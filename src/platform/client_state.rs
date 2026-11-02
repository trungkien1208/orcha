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
