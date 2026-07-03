use crate::profile::types::ProfilesConfig;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),
    #[error("JSON serialization error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Config directory not found")]
    ConfigDirNotFound,
}

pub type Result<T> = std::result::Result<T, StorageError>;

/// Get the legacy config file path (slack-cli) for migration purposes
fn legacy_config_path() -> Result<PathBuf> {
    directories::ProjectDirs::from("", "", "slack-cli")
        .map(|dirs| dirs.config_dir().join("profiles.json"))
        .ok_or(StorageError::ConfigDirNotFound)
}

/// Get the default config file path using the OS config directory
pub fn default_config_path() -> Result<PathBuf> {
    // Check for environment variable override (used in testing)
    if let Ok(config_path) = std::env::var("SLACK_RS_CONFIG_PATH") {
        return Ok(PathBuf::from(config_path));
    }

    let dirs = directories::ProjectDirs::from("", "", "slack-rs")
        .ok_or(StorageError::ConfigDirNotFound)?;
    let config_dir = dirs.config_dir();

    // Create directory if it doesn't exist
    fs::create_dir_all(config_dir)?;

    Ok(config_dir.join("profiles.json"))
}

/// Migrate legacy config file to new path if needed
/// This function is only called when using the default config path
fn migrate_legacy_config_internal() -> Result<bool> {
    // Get new default path
    let new_path = default_config_path()?;

    // If new path already exists, no migration needed
    if new_path.exists() {
        return Ok(false);
    }

    // Try to get legacy path
    let legacy_path = match legacy_config_path() {
        Ok(path) => path,
        Err(_) => return Ok(false),
    };

    // If legacy path doesn't exist, no migration needed
    if !legacy_path.exists() {
        return Ok(false);
    }

    // Create parent directory for new path if it doesn't exist
    if let Some(parent) = new_path.parent() {
        fs::create_dir_all(parent)?;
    }

    // Try to rename (move) the file first
    match fs::rename(&legacy_path, &new_path) {
        Ok(_) => Ok(true),
        Err(_) => {
            // If rename fails (e.g., different filesystems), copy and keep the old file
            let content = fs::read_to_string(&legacy_path)?;
            fs::write(&new_path, content)?;
            Ok(true)
        }
    }
}

/// Migrate legacy config file for a specific path (used for testing)
/// Returns true if migration was performed
#[cfg(test)]
fn migrate_legacy_config(legacy_path: &Path, new_path: &Path) -> Result<bool> {
    // If new path already exists, no migration needed
    if new_path.exists() {
        return Ok(false);
    }

    // If legacy path doesn't exist, no migration needed
    if !legacy_path.exists() {
        return Ok(false);
    }

    // Create parent directory for new path if it doesn't exist
    if let Some(parent) = new_path.parent() {
        fs::create_dir_all(parent)?;
    }

    // Try to rename (move) the file first
    match fs::rename(legacy_path, new_path) {
        Ok(_) => Ok(true),
        Err(_) => {
            // If rename fails (e.g., different filesystems), copy and keep the old file
            let content = fs::read_to_string(legacy_path)?;
            fs::write(new_path, content)?;
            Ok(true)
        }
    }
}

/// Load profiles config from a file
pub fn load_config(path: &Path) -> Result<ProfilesConfig> {
    // Try to migrate legacy config if this is the default path
    // Only attempt migration when using default_config_path
    if let Ok(default_path) = default_config_path() {
        if path == default_path {
            let _ = migrate_legacy_config_internal();
        }
    }

    if !path.exists() {
        return Ok(ProfilesConfig::new());
    }

    // If the existing file is readable by group/other, tighten to 0600.
    tighten_permissions(path);

    let content = fs::read_to_string(path)?;
    let config: ProfilesConfig = serde_json::from_str(&content)?;
    Ok(config)
}

/// Tighten permissions of an existing file to 0600 if group/other bits are set
/// (Unix only; best-effort — loading proceeds even if tightening fails)
#[cfg(unix)]
fn tighten_permissions(path: &Path) {
    use std::os::unix::fs::PermissionsExt;

    if let Ok(metadata) = fs::metadata(path) {
        let mode = metadata.permissions().mode();
        if mode & 0o077 != 0 {
            let mut permissions = metadata.permissions();
            permissions.set_mode(0o600);
            let _ = fs::set_permissions(path, permissions);
        }
    }
}

#[cfg(not(unix))]
fn tighten_permissions(_path: &Path) {}

/// Save profiles config to a file
///
/// The file is created atomically with 0600 permissions on Unix: content is
/// written to a temp file in the same directory (created with mode 0600) and
/// then renamed over the target, so the config is never world-readable — not
/// even briefly — and never partially written.
pub fn save_config(path: &Path, config: &ProfilesConfig) -> Result<()> {
    // Create parent directory if it doesn't exist
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let content = serde_json::to_string_pretty(config)?;
    write_atomic_0600(path, content.as_bytes())?;
    Ok(())
}

/// Atomically write `data` to `path` with 0600 permissions (Unix)
///
/// Shared by profile config storage and export/import secure file writes.
#[cfg(unix)]
pub(crate) fn write_atomic_0600(path: &Path, data: &[u8]) -> io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;

    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "config".to_string());

    // Create a unique temp file in the same directory with 0600 from the start
    let mut last_err = None;
    for _ in 0..16 {
        let tmp_path = dir.join(format!(".{}.tmp.{:08x}", file_name, rand::random::<u32>()));
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp_path)
        {
            Ok(mut file) => {
                if let Err(e) = file.write_all(data).and_then(|_| file.sync_all()) {
                    let _ = fs::remove_file(&tmp_path);
                    return Err(e);
                }
                drop(file);
                if let Err(e) = fs::rename(&tmp_path, path) {
                    let _ = fs::remove_file(&tmp_path);
                    return Err(e);
                }
                return Ok(());
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                last_err = Some(e);
                continue;
            }
            Err(e) => return Err(e),
        }
    }
    Err(last_err.unwrap_or_else(|| io::Error::other("failed to create unique temp file")))
}

/// Atomically write `data` to `path` (non-Unix: no permission handling)
#[cfg(not(unix))]
pub(crate) fn write_atomic_0600(path: &Path, data: &[u8]) -> io::Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "config".to_string());
    let tmp_path = dir.join(format!(".{}.tmp.{:08x}", file_name, rand::random::<u32>()));
    fs::write(&tmp_path, data)?;
    if let Err(e) = fs::rename(&tmp_path, path) {
        let _ = fs::remove_file(&tmp_path);
        return Err(e);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::types::Profile;
    use tempfile::TempDir;

    #[test]
    fn test_save_and_load_config() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("profiles.json");

        let mut config = ProfilesConfig::new();
        config.set(
            "default".to_string(),
            Profile {
                team_id: "T123".to_string(),
                user_id: "U456".to_string(),
                team_name: Some("Test Team".to_string()),
                user_name: Some("Test User".to_string()),
                client_id: None,
                redirect_uri: None,
                scopes: None,
                bot_scopes: None,
                user_scopes: None,
                default_token_type: None,
            },
        );

        // Save config
        save_config(&config_path, &config).unwrap();
        assert!(config_path.exists());

        // Load config
        let loaded = load_config(&config_path).unwrap();
        assert_eq!(config, loaded);
    }

    #[test]
    fn test_load_nonexistent_config() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("nonexistent.json");

        let loaded = load_config(&config_path).unwrap();
        assert_eq!(loaded, ProfilesConfig::new());
    }

    #[test]
    fn test_save_creates_parent_directory() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("nested/dir/profiles.json");

        let config = ProfilesConfig::new();
        save_config(&config_path, &config).unwrap();

        assert!(config_path.exists());
        assert!(config_path.parent().unwrap().exists());
    }

    #[test]
    fn test_load_save_round_trip() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("profiles.json");

        let mut config = ProfilesConfig::new();
        config.set(
            "profile1".to_string(),
            Profile {
                team_id: "T1".to_string(),
                user_id: "U1".to_string(),
                team_name: None,
                user_name: None,
                client_id: None,
                redirect_uri: None,
                scopes: None,
                bot_scopes: None,
                user_scopes: None,
                default_token_type: None,
            },
        );
        config.set(
            "profile2".to_string(),
            Profile {
                team_id: "T2".to_string(),
                user_id: "U2".to_string(),
                team_name: Some("Team 2".to_string()),
                user_name: Some("User 2".to_string()),
                client_id: None,
                redirect_uri: None,
                scopes: None,
                bot_scopes: None,
                user_scopes: None,
                default_token_type: None,
            },
        );

        save_config(&config_path, &config).unwrap();
        let loaded = load_config(&config_path).unwrap();
        assert_eq!(config, loaded);
    }

    #[test]
    fn test_default_config_path() {
        // Just verify it doesn't panic and returns something
        let result = default_config_path();
        match result {
            Ok(path) => {
                assert!(path.to_string_lossy().contains("slack-rs"));
                assert!(path.to_string_lossy().contains("profiles.json"));
            }
            Err(StorageError::ConfigDirNotFound) => {
                // This might happen in some test environments
            }
            Err(e) => panic!("Unexpected error: {}", e),
        }
    }

    #[test]
    fn test_migrate_legacy_config_path() {
        let temp_dir = TempDir::new().unwrap();
        let legacy_path = temp_dir.path().join("legacy").join("profiles.json");
        let new_path = temp_dir.path().join("new").join("profiles.json");

        // Create legacy config
        let mut config = ProfilesConfig::new();
        config.set(
            "legacy".to_string(),
            Profile {
                team_id: "T123".to_string(),
                user_id: "U456".to_string(),
                team_name: Some("Legacy Team".to_string()),
                user_name: Some("Legacy User".to_string()),
                client_id: None,
                redirect_uri: None,
                scopes: None,
                bot_scopes: None,
                user_scopes: None,
                default_token_type: None,
            },
        );
        fs::create_dir_all(legacy_path.parent().unwrap()).unwrap();
        save_config(&legacy_path, &config).unwrap();
        assert!(legacy_path.exists());

        // Perform migration
        let migrated = migrate_legacy_config(&legacy_path, &new_path).unwrap();
        assert!(migrated);
        assert!(new_path.exists());

        // Verify migrated content
        let loaded = load_config(&new_path).unwrap();
        assert_eq!(config, loaded);

        // Test that migration is skipped if new path exists
        let legacy_path2 = temp_dir.path().join("legacy2").join("profiles.json");
        fs::create_dir_all(legacy_path2.parent().unwrap()).unwrap();
        save_config(&legacy_path2, &config).unwrap();

        let migrated_again = migrate_legacy_config(&legacy_path2, &new_path).unwrap();
        assert!(!migrated_again);
    }

    #[test]
    fn test_load_config_with_migration() {
        let temp_dir = TempDir::new().unwrap();
        let legacy_path = temp_dir.path().join("legacy").join("profiles.json");
        let new_path = temp_dir.path().join("new").join("profiles.json");

        // Create legacy config
        let mut config = ProfilesConfig::new();
        config.set(
            "test".to_string(),
            Profile {
                team_id: "T999".to_string(),
                user_id: "U888".to_string(),
                team_name: None,
                user_name: None,
                client_id: None,
                redirect_uri: None,
                scopes: None,
                bot_scopes: None,
                user_scopes: None,
                default_token_type: None,
            },
        );
        fs::create_dir_all(legacy_path.parent().unwrap()).unwrap();
        save_config(&legacy_path, &config).unwrap();

        // Manually trigger migration by calling migrate_legacy_config
        migrate_legacy_config(&legacy_path, &new_path).unwrap();

        // Load from new path should work
        let loaded = load_config(&new_path).unwrap();
        assert_eq!(config, loaded);
    }

    #[cfg(unix)]
    #[test]
    fn test_save_config_creates_file_with_0600() {
        use std::os::unix::fs::PermissionsExt;

        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("profiles.json");

        let config = ProfilesConfig::new();
        save_config(&config_path, &config).unwrap();

        let mode = fs::metadata(&config_path).unwrap().permissions().mode();
        assert_eq!(
            mode & 0o777,
            0o600,
            "profiles.json should be created with 0600, got {:o}",
            mode & 0o777
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_save_config_overwrite_keeps_0600_and_content() {
        use std::os::unix::fs::PermissionsExt;

        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("profiles.json");

        let mut config = ProfilesConfig::new();
        save_config(&config_path, &config).unwrap();

        config.set(
            "p1".to_string(),
            Profile {
                team_id: "T1".to_string(),
                user_id: "U1".to_string(),
                team_name: None,
                user_name: None,
                client_id: None,
                redirect_uri: None,
                scopes: None,
                bot_scopes: None,
                user_scopes: None,
                default_token_type: None,
            },
        );
        save_config(&config_path, &config).unwrap();

        let mode = fs::metadata(&config_path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);

        let loaded = load_config(&config_path).unwrap();
        assert_eq!(config, loaded);

        // No temp files left behind
        let leftovers: Vec<_> = fs::read_dir(temp_dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp."))
            .collect();
        assert!(leftovers.is_empty(), "temp files left behind");
    }

    #[cfg(unix)]
    #[test]
    fn test_load_config_tightens_loose_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("profiles.json");

        let config = ProfilesConfig::new();
        save_config(&config_path, &config).unwrap();

        // Loosen permissions to simulate a legacy/world-readable file
        let mut permissions = fs::metadata(&config_path).unwrap().permissions();
        permissions.set_mode(0o644);
        fs::set_permissions(&config_path, permissions).unwrap();

        // Loading must succeed and tighten to 0600
        let loaded = load_config(&config_path).unwrap();
        assert_eq!(config, loaded);

        let mode = fs::metadata(&config_path).unwrap().permissions().mode();
        assert_eq!(
            mode & 0o777,
            0o600,
            "load_config should tighten loose permissions to 0600"
        );
    }
}
