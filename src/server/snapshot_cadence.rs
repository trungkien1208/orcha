//! Connection-local pacing for passive shell metadata. Never delays runtime
//! state, API replies, notifications, pane output, or endpoint lease changes.
use std::time::{Duration, Instant};

const PASSIVE_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Debug, Default)]
pub(crate) struct SnapshotCadence {
    checked_at: Option<Instant>,
    deferred: bool,
    forced: bool,
}

impl SnapshotCadence {
    pub(crate) fn invalidate(&mut self) {
        // Preserve an outstanding trailing flush. An input/transport event
        // may itself produce no render, so dropping this deadline would lose
        // the last metadata update forever when output has stopped.
        self.forced = true;
    }

    pub(crate) fn should_refresh(&mut self, now: Instant) -> bool {
        let due = self.forced
            || self
                .checked_at
                .is_none_or(|checked| now.saturating_duration_since(checked) >= PASSIVE_INTERVAL);
        self.deferred = !due;
        due
    }

    pub(crate) fn refreshed(&mut self, now: Instant) {
        self.checked_at = Some(now);
        self.deferred = false;
        self.forced = false;
    }

    /// A synchronized screen cannot publish a coherent surface yet. Keep
    /// the trailing update, but retry on a bounded timer instead of spinning
    /// on an already expired deadline.
    pub(crate) fn retry_later(&mut self, now: Instant) {
        self.checked_at = Some(now);
        self.deferred = true;
        self.forced = false;
    }

    pub(crate) fn deadline(&self) -> Option<Instant> {
        self.checked_at.filter(|_| self.deferred).map(|checked| {
            if self.forced {
                checked
            } else {
                checked + PASSIVE_INTERVAL
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bursts_have_a_fixed_trailing_deadline_and_do_not_postpone_it() {
        let now = Instant::now();
        let mut cadence = SnapshotCadence::default();
        assert!(cadence.should_refresh(now));
        cadence.refreshed(now);
        assert_eq!(cadence.deadline(), None);
        for ms in [1, 20, 50, 99] {
            assert!(!cadence.should_refresh(now + Duration::from_millis(ms)));
            assert_eq!(cadence.deadline(), Some(now + PASSIVE_INTERVAL));
        }
        assert!(cadence.should_refresh(now + PASSIVE_INTERVAL));
        cadence.refreshed(now + PASSIVE_INTERVAL);
        assert_eq!(cadence.deadline(), None);
    }

    #[test]
    fn blocked_coherent_render_keeps_a_bounded_retry() {
        let now = Instant::now();
        let mut cadence = SnapshotCadence::default();
        cadence.refreshed(now);
        assert!(!cadence.should_refresh(now + Duration::from_millis(1)));
        cadence.retry_later(now + PASSIVE_INTERVAL);
        assert_eq!(cadence.deadline(), Some(now + 2 * PASSIVE_INTERVAL));
        assert!(!cadence.should_refresh(now + PASSIVE_INTERVAL + Duration::from_millis(1)));
    }

    #[test]
    fn urgent_changes_and_new_connections_bypass_an_existing_burst() {
        let now = Instant::now();
        let mut cadence = SnapshotCadence::default();
        cadence.refreshed(now);
        assert!(!cadence.should_refresh(now + Duration::from_millis(1)));
        cadence.invalidate();
        assert_eq!(cadence.deadline(), Some(now));
        assert!(cadence.should_refresh(now + Duration::from_millis(2)));
        let mut other = SnapshotCadence::default();
        assert!(other.should_refresh(now));
    }
}
