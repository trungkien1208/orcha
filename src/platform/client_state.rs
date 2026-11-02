use std::path::Path;

#[cfg(not(windows))]
pub(crate) fn create_private_state_file(path: &Path) -> std::io::Result<std::fs::File> {
    super::create_remote_ssh_config_file(path)
}

#[cfg(windows)]
pub(crate) fn create_private_state_file(path: &Path) -> std::io::Result<std::fs::File> {
    super::windows::create_remote_ssh_config_file(path)
}

#[cfg(not(windows))]
pub(crate) fn replace_file(source: &Path, destination: &Path) -> std::io::Result<()> {
    std::fs::rename(source, destination)
}

#[cfg(windows)]
pub(crate) fn replace_file(source: &Path, destination: &Path) -> std::io::Result<()> {
    super::windows::replace_file(source, destination)
}

#[cfg(not(windows))]
pub(crate) fn sync_parent_directory(path: &Path) -> std::io::Result<()> {
    std::fs::File::open(path)?.sync_all()
}

#[cfg(windows)]
pub(crate) fn sync_parent_directory(_path: &Path) -> std::io::Result<()> {
    // replace_file uses MOVEFILE_WRITE_THROUGH on Windows.
    Ok(())
}

/// 16 bytes from the operating system's random source, for secrets such as
/// the settings page's token.
#[cfg(not(windows))]
pub(crate) fn random_secret() -> std::io::Result<[u8; 16]> {
    use std::io::Read;
    let mut bytes = [0u8; 16];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(bytes)
}

/// 16 bytes from the operating system's random source, for secrets such as
/// the settings page's token: a version-4 GUID, whose 122 random bits come
/// from the system RNG (the other 6 mark its version).
#[cfg(windows)]
pub(crate) fn random_secret() -> std::io::Result<[u8; 16]> {
    // SAFETY: CoCreateGuid takes no pointers and needs no COM apartment.
    let guid =
        unsafe { windows::Win32::System::Com::CoCreateGuid() }.map_err(std::io::Error::other)?;
    Ok(guid.to_u128().to_ne_bytes())
}

/// Restricts an existing state file or directory to its owner. Refuse
/// symlinks so migrating old state never changes an unrelated target.
pub(crate) fn restrict_private_state_path(path: &Path) -> std::io::Result<()> {
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || (!metadata.is_dir() && !metadata.is_file()) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "private state must be a regular file or directory",
        ));
    }
    restrict_private_state_permissions(path, metadata.is_dir())
}

#[cfg(unix)]
fn restrict_private_state_permissions(path: &Path, directory: bool) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(
        path,
        std::fs::Permissions::from_mode(if directory { 0o700 } else { 0o600 }),
    )
}

#[cfg(windows)]
fn restrict_private_state_permissions(path: &Path, _directory: bool) -> std::io::Result<()> {
    super::windows::restrict_private_state_permissions(path)
}

#[cfg(not(any(unix, windows)))]
fn restrict_private_state_permissions(_path: &Path, _directory: bool) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "this platform cannot restrict private state permissions",
    ))
}
