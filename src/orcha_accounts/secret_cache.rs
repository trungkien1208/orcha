//! Short-lived, per-service reads for passive fuel/sign-in polling. Mutations
//! still read their source directly and invalidate before and after a write.
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use crate::platform::OrchaSecretSlot;

const PRESENT_TTL: Duration = Duration::from_secs(120);
const MISSING_TTL: Duration = Duration::from_secs(5);
const MAX_SLOTS: usize = 128;

type Slot = Arc<Mutex<Option<Entry>>>;
struct Entry {
    at: Instant,
    value: Option<String>,
}

impl Entry {
    fn fresh(&self, now: Instant) -> bool {
        let ttl = if self.value.is_some() {
            PRESENT_TTL
        } else {
            MISSING_TTL
        };
        now.checked_duration_since(self.at)
            .is_some_and(|age| age < ttl)
    }
}

#[derive(Default)]
struct Cache {
    slots: Mutex<HashMap<String, Slot>>,
}

impl Cache {
    fn read_with(
        &self,
        key: String,
        now: Instant,
        read: impl FnOnce() -> Option<String>,
    ) -> Option<String> {
        let slot = {
            let mut slots = self
                .slots
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if slots.len() >= MAX_SLOTS && !slots.contains_key(&key) {
                slots.clear();
            }
            // Expired sign-ins leave memory now, not when their service is
            // read again. A slot busy with a read is left for that read.
            slots.retain(|_, slot| {
                slot.try_lock()
                    .map(|entry| entry.as_ref().is_some_and(|entry| entry.fresh(now)))
                    .unwrap_or(true)
            });
            slots.entry(key).or_default().clone()
        };
        // A slow Keychain call blocks only this service, not other accounts.
        let mut entry = slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(cached) = entry.as_ref().filter(|cached| cached.fresh(now)) {
            return cached.value.clone();
        }
        let value = read();
        *entry = Some(Entry {
            at: now,
            value: value.clone(),
        });
        value
    }

    fn invalidate(&self) {
        // An in-flight read owns its old slot; it cannot reinsert that value
        // into this map after a mutation has invalidated it.
        self.slots
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();
    }

    fn write_with<T>(&self, write: impl FnOnce() -> T) -> T {
        self.invalidate();
        let result = write();
        self.invalidate();
        result
    }
}

static CACHE: LazyLock<Cache> = LazyLock::new(Cache::default);

pub(super) fn read(slot: &OrchaSecretSlot) -> Option<String> {
    // Home/config identity matters for kept files on hosts without Keychain.
    let key = format!(
        "{:?}:{:?}:{}",
        crate::platform::user_home_dir(),
        crate::platform::orcha_config_base_dir(),
        crate::platform::orcha_claude_service(slot)
    );
    CACHE.read_with(key, Instant::now(), || {
        crate::platform::orcha_secret_read(slot)
    })
}

pub(super) fn invalidate_all() {
    CACHE.invalidate();
}

pub(super) fn write(
    slot: &OrchaSecretSlot,
    secret: &str,
    from: &OrchaSecretSlot,
) -> std::io::Result<()> {
    CACHE.write_with(|| crate::platform::orcha_secret_write(slot, secret, from))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn expired_sign_ins_leave_memory_when_any_service_is_read() {
        let cache = Cache::default();
        let now = Instant::now();
        assert!(cache
            .read_with("work".into(), now, || Some("fixture-sign-in".into()))
            .is_some());
        cache.read_with("personal".into(), now + PRESENT_TTL, || None);
        let slots = cache
            .slots
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        assert!(!slots.contains_key("work"), "the expired sign-in is gone");
    }

    #[test]
    fn repeated_polling_reads_each_service_once_until_expiry() {
        let cache = Cache::default();
        let now = Instant::now();
        let calls = Cell::new(0);
        let read = || {
            calls.set(calls.get() + 1);
            Some("fixture-sign-in".into())
        };
        assert!(cache.read_with("work".into(), now, read).is_some());
        assert!(cache
            .read_with("work".into(), now + Duration::from_secs(60), read)
            .is_some());
        assert_eq!(calls.get(), 1);
        assert!(cache.read_with("personal".into(), now, read).is_some());
        assert_eq!(calls.get(), 2, "different services never share a sign-in");
        assert!(cache
            .read_with("work".into(), now + PRESENT_TTL, read)
            .is_some());
        assert_eq!(calls.get(), 3);
    }

    #[test]
    fn missing_sign_in_is_checked_again_quickly_and_explicit_refresh_invalidates() {
        let cache = Cache::default();
        let now = Instant::now();
        assert!(cache.read_with("work".into(), now, || None).is_none());
        assert!(cache
            .read_with("work".into(), now + Duration::from_secs(1), || Some(
                "new".into()
            ))
            .is_none());
        assert_eq!(
            cache.read_with("work".into(), now + MISSING_TTL, || Some("new".into())),
            Some("new".into())
        );
        cache.invalidate();
        assert!(cache
            .read_with("work".into(), now + MISSING_TTL, || None)
            .is_none());
    }

    #[test]
    fn successful_or_failed_writes_drop_old_and_during_write_reads() {
        let cache = Cache::default();
        let now = Instant::now();
        for succeeds in [false, true] {
            cache.read_with("work".into(), now, || Some("old".into()));
            let result: Result<(), ()> = cache.write_with(|| {
                assert_eq!(
                    cache.read_with("work".into(), now, || Some("during".into())),
                    Some("during".into())
                );
                if succeeds {
                    Ok(())
                } else {
                    Err(())
                }
            });
            assert_eq!(result.is_ok(), succeeds);
            assert_eq!(
                cache.read_with("work".into(), now, || Some("after".into())),
                Some("after".into())
            );
        }
    }
}
