//! Small JSON state files in orcha's config folder: answers to offers,
//! notices already shown, project-rule choices and update caches. One way
//! to read and write them:
//!
//! - a missing (or empty) file is the default;
//! - a file that cannot be read or is not valid JSON is logged, read as the
//!   default and left as it is: `update` refuses to write over it, so a
//!   hand-edited mistake is never lost (only `save`, for caches that are
//!   rebuilt anyway, replaces it);
//! - writes are atomic and locked, so two orcha windows cannot interleave.

use std::path::Path;

use serde::{de::DeserializeOwned, Serialize};

/// Seconds since the Unix epoch (0 when the clock is before it): the
/// stamp in orcha's caches.
pub(crate) fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

/// The value in `path`; the default when the file is missing, unreadable
/// or not valid JSON (the last two are logged and the file is kept).
pub(crate) fn load_or_default<T: DeserializeOwned + Default>(path: &Path) -> T {
    match load(path) {
        Ok(value) => value.unwrap_or_default(),
        Err(err) => {
            tracing::warn!(%err, "orcha state file ignored");
            T::default()
        }
    }
}

/// None when `path` is missing or empty; an error when it cannot be read
/// or does not hold a `T`.
fn load<T: DeserializeOwned>(path: &Path) -> Result<Option<T>, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(format!("could not read {}: {err}", path.display())),
    };
    if text.trim().is_empty() {
        return Ok(None);
    }
    serde_json::from_str(&text).map(Some).map_err(|err| {
        format!(
            "{} is not valid ({err}), so orcha left it as it is",
            path.display()
        )
    })
}

fn to_text<T: Serialize>(value: &T) -> Result<String, String> {
    serde_json::to_string_pretty(value)
        .map(|text| text + "\n")
        .map_err(|err| err.to_string())
}

fn lock(path: &Path) -> Result<std::fs::File, String> {
    crate::orcha_app::lock_for_update(path)
        .map_err(|err| format!("could not lock {}: {err}", path.display()))
}

/// Replaces `path` with `value`, whatever it held. For caches only: they
/// are rebuilt from elsewhere, so a broken one is simply replaced.
pub(crate) fn save<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let text = to_text(value)?;
    let _lock = lock(path)?;
    crate::orcha_app::write_atomic(path, &text)
        .map_err(|err| format!("could not save {}: {err}", path.display()))
}

/// Reads `path` (the default when missing), lets `change` edit the value
/// and writes it back when it changed, all under the file's lock. A file
/// that cannot be read or is not valid is left alone and nothing changes.
pub(crate) fn update<T, R>(path: &Path, change: impl FnOnce(&mut T) -> R) -> Result<R, String>
where
    T: Serialize + DeserializeOwned + Default,
{
    let _lock = lock(path)?;
    let loaded: Option<T> = load(path)?;
    let existed = loaded.is_some();
    let mut value = loaded.unwrap_or_default();
    let before = to_text(&value)?;
    let out = change(&mut value);
    let after = to_text(&value)?;
    if after != before || !existed {
        crate::orcha_app::write_atomic(path, &after)
            .map_err(|err| format!("could not save {}: {err}", path.display()))?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("orcha-state-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("test folder");
        dir
    }

    #[test]
    fn a_missing_file_is_the_default_and_updates_create_it() {
        let dir = temp_dir("missing");
        let path = dir.join("answers.json");
        let answers: HashMap<String, String> = load_or_default(&path);
        assert!(answers.is_empty());
        let added = update(&path, |answers: &mut HashMap<String, String>| {
            answers.insert("easy-job".into(), "never".into())
        });
        assert_eq!(added, Ok(None));
        let answers: HashMap<String, String> = load_or_default(&path);
        assert_eq!(answers["easy-job"], "never");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_corrupt_file_reads_as_the_default_and_is_never_written_over() {
        let dir = temp_dir("corrupt");
        let path = dir.join("answers.json");
        std::fs::write(&path, "{\"easy-job\": \"nev").unwrap();
        let answers: HashMap<String, String> = load_or_default(&path);
        assert!(answers.is_empty());
        let refused = update(&path, |answers: &mut HashMap<String, String>| {
            answers.insert("other".into(), "0.1.0".into());
        });
        assert!(refused.is_err_and(|err| err.contains("left it as it is")));
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "{\"easy-job\": \"nev"
        );
        // a cache, rebuilt anyway, is replaced
        save(&path, &HashMap::from([("at".to_owned(), 1)])).expect("saved");
        let cache: HashMap<String, u64> = load_or_default(&path);
        assert_eq!(cache["at"], 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_unchanged_value_is_not_written_again() {
        let dir = temp_dir("unchanged");
        let path = dir.join("notices.json");
        update(&path, |shown: &mut HashMap<String, String>| {
            shown.insert("a".into(), "1".into());
        })
        .expect("first");
        let written = std::fs::metadata(&path).unwrap().modified().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        update(&path, |shown: &mut HashMap<String, String>| {
            shown.get("a").cloned()
        })
        .expect("read only");
        assert_eq!(
            std::fs::metadata(&path).unwrap().modified().unwrap(),
            written
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
