use std::path::PathBuf;

const BRIDGE_APP_FOLDER_NAME: &str = "karlsen-stratum-bridge";

fn get_home_dir() -> PathBuf {
    #[cfg(target_os = "windows")]
    return dirs::data_local_dir().or_else(dirs::home_dir).unwrap_or_else(|| PathBuf::from(".")); // Fallback to current directory

    #[cfg(not(target_os = "windows"))]
    return dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")); // Fallback to current directory
}

pub(crate) fn get_bridge_app_dir() -> PathBuf {
    #[cfg(target_os = "windows")]
    return get_home_dir().join(BRIDGE_APP_FOLDER_NAME);
    #[cfg(not(target_os = "windows"))]
    return get_home_dir().join(format!(".{}", BRIDGE_APP_FOLDER_NAME));
}

/// Directory for `bridge.log`: the folder that contains the executable (like the Go bridge),
/// so logs sit next to the binary on Windows, Linux and HiveOS. Falls back to the per-user
/// app directory when the executable's folder cannot be written.
pub(crate) fn get_bridge_logs_dir() -> PathBuf {
    if let Some(dir) = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf())) {
        let probe = dir.join(".bridge-write-test");
        if std::fs::write(&probe, b"").is_ok() {
            let _ = std::fs::remove_file(&probe);
            return dir;
        }
    }
    let fallback = get_bridge_app_dir().join("logs");
    let _ = std::fs::create_dir_all(&fallback);
    fallback
}
