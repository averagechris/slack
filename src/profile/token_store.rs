//! Token storage backed by the OS keyring.
//!
//! Secrets (bot tokens, user tokens, OAuth client secrets) are stored
//! exclusively in the platform credential store (macOS Keychain, Windows
//! Credential Manager, Linux Secret Service). There is no plaintext file
//! persistence; the legacy `tokens.json` file is only read by the one-time
//! `auth migrate` command and then securely deleted.
//!
//! Non-secret profile metadata continues to live in `profiles.json`
//! (see [`crate::profile::storage`]), which also serves as the enumeration
//! index for profiles since the keyring cannot list entries.

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use thiserror::Error;

/// Keyring service name for all entries created by this CLI.
///
/// Kept in sync with `program_name` in `config/cli.toml` (the single source
/// of truth for the binary name).
pub const KEYRING_SERVICE: &str = "slack";

#[derive(Debug, Error)]
pub enum TokenStoreError {
    #[error("Token not found for key: {0}")]
    NotFound(String),
    #[error("Failed to store token: {0}")]
    StoreFailed(String),
    #[error("Failed to delete token: {0}")]
    DeleteFailed(String),
    #[error("IO error: {0}")]
    IoError(String),
    #[error(
        "OS keyring unavailable: {0}\n\
         On macOS this is the Keychain; on Windows the Credential Manager.\n\
         On Linux a Secret Service provider (e.g. gnome-keyring or KWallet)\n\
         must be installed, unlocked, and reachable on the session D-Bus.\n\
         Headless hosts typically need `gnome-keyring-daemon` started with\n\
         the login session (or a D-Bus session via `dbus-run-session`)."
    )]
    KeyringUnavailable(String),
}

pub type Result<T> = std::result::Result<T, TokenStoreError>;

/// Trait for storing and retrieving tokens securely
pub trait TokenStore: Send + Sync {
    /// Store a token with the given key
    fn set(&self, key: &str, token: &str) -> Result<()>;

    /// Retrieve a token by key
    fn get(&self, key: &str) -> Result<String>;

    /// Delete a token by key
    fn delete(&self, key: &str) -> Result<()>;

    /// Check if a token exists for the given key
    fn exists(&self, key: &str) -> bool;
}

/// In-memory implementation of TokenStore for testing
#[derive(Debug, Clone)]
pub struct InMemoryTokenStore {
    tokens: Arc<Mutex<HashMap<String, String>>>,
}

impl InMemoryTokenStore {
    pub fn new() -> Self {
        Self {
            tokens: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl Default for InMemoryTokenStore {
    fn default() -> Self {
        Self::new()
    }
}

impl TokenStore for InMemoryTokenStore {
    fn set(&self, key: &str, token: &str) -> Result<()> {
        let mut tokens = self.tokens.lock().unwrap();
        tokens.insert(key.to_string(), token.to_string());
        Ok(())
    }

    fn get(&self, key: &str) -> Result<String> {
        let tokens = self.tokens.lock().unwrap();
        tokens
            .get(key)
            .cloned()
            .ok_or_else(|| TokenStoreError::NotFound(key.to_string()))
    }

    fn delete(&self, key: &str) -> Result<()> {
        let mut tokens = self.tokens.lock().unwrap();
        tokens
            .remove(key)
            .ok_or_else(|| TokenStoreError::NotFound(key.to_string()))?;
        Ok(())
    }

    fn exists(&self, key: &str) -> bool {
        let tokens = self.tokens.lock().unwrap();
        tokens.contains_key(key)
    }
}

/// OS-keyring-backed implementation of TokenStore.
///
/// Entry layout: one keyring entry per credential set, holding a JSON map
/// of token-store keys to secret values (the same flat key format the
/// legacy `tokens.json` used, so encrypted export/import round-trips
/// unchanged):
///
/// - Bot and user tokens for a profile identity share one entry with the
///   account name `{team_id}:{user_id}` (keys `T:U` and `T:U:user`).
/// - OAuth client secrets get their own entry with the account name
///   `oauth-client-secret:{profile_name}`.
///
/// The keyring cannot enumerate entries; `profiles.json` remains the index
/// of profiles, from which every account name above can be derived.
#[derive(Debug, Clone, Default)]
pub struct KeyringTokenStore;

impl KeyringTokenStore {
    /// Create a new keyring-backed token store.
    pub fn new() -> Result<Self> {
        // Test-only escape hatch: debug builds honor SLACK_KEYRING_MOCK so
        // integration tests (including ones that spawn the binary) never
        // touch the real OS credential store. Release builds ignore it.
        #[cfg(debug_assertions)]
        if std::env::var("SLACK_KEYRING_MOCK").is_ok() {
            use_mock_keyring();
        }
        Ok(Self)
    }

    /// Group token-store keys into keyring account names.
    ///
    /// `{team}:{user}` and `{team}:{user}:user` map to the same account so a
    /// profile identity's bot and user tokens share one keyring entry.
    fn account_for_key(key: &str) -> String {
        if let Some(base) = key.strip_suffix(":user") {
            if base.contains(':') && !base.starts_with("oauth-client-secret") {
                return base.to_string();
            }
        }
        key.to_string()
    }

    /// Get (or create) the keyring entry handle for an account.
    ///
    /// Handles are cached process-wide: for the real platform stores an
    /// `Entry` is just a handle (state lives in the OS store), and caching
    /// makes keyring's mock store — which keeps state inside the `Entry` —
    /// behave consistently across store instances in tests.
    fn entry(account: &str) -> Result<Arc<keyring::Entry>> {
        use std::sync::OnceLock;
        static CACHE: OnceLock<Mutex<HashMap<String, Arc<keyring::Entry>>>> = OnceLock::new();
        let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
        let mut map = cache.lock().unwrap();
        if let Some(entry) = map.get(account) {
            return Ok(Arc::clone(entry));
        }
        let entry = Arc::new(
            keyring::Entry::new(KEYRING_SERVICE, account)
                .map_err(|e| map_keyring_error("open keyring entry", account, e))?,
        );
        map.insert(account.to_string(), Arc::clone(&entry));
        Ok(entry)
    }

    /// Load the JSON blob for an account; missing entry yields an empty map.
    fn load_blob(account: &str) -> Result<BTreeMap<String, String>> {
        let entry = Self::entry(account)?;
        match entry.get_password() {
            Ok(raw) => serde_json::from_str(&raw).map_err(|e| {
                TokenStoreError::StoreFailed(format!("Corrupt keyring entry '{}': {}", account, e))
            }),
            Err(keyring::Error::NoEntry) => Ok(BTreeMap::new()),
            Err(e) => Err(map_keyring_error("read keyring entry", account, e)),
        }
    }

    fn save_blob(account: &str, blob: &BTreeMap<String, String>) -> Result<()> {
        let entry = Self::entry(account)?;
        if blob.is_empty() {
            match entry.delete_credential() {
                Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
                Err(e) => Err(map_keyring_error("delete keyring entry", account, e)),
            }
        } else {
            let raw = serde_json::to_string(blob).map_err(|e| {
                TokenStoreError::StoreFailed(format!("Failed to serialize tokens: {}", e))
            })?;
            entry
                .set_password(&raw)
                .map_err(|e| map_keyring_error("write keyring entry", account, e))
        }
    }
}

impl TokenStore for KeyringTokenStore {
    fn set(&self, key: &str, token: &str) -> Result<()> {
        let account = Self::account_for_key(key);
        let mut blob = Self::load_blob(&account)?;
        blob.insert(key.to_string(), token.to_string());
        Self::save_blob(&account, &blob)
    }

    fn get(&self, key: &str) -> Result<String> {
        let account = Self::account_for_key(key);
        Self::load_blob(&account)?
            .get(key)
            .cloned()
            .ok_or_else(|| TokenStoreError::NotFound(key.to_string()))
    }

    fn delete(&self, key: &str) -> Result<()> {
        let account = Self::account_for_key(key);
        let mut blob = Self::load_blob(&account)?;
        if blob.remove(key).is_none() {
            return Err(TokenStoreError::NotFound(key.to_string()));
        }
        Self::save_blob(&account, &blob)
    }

    fn exists(&self, key: &str) -> bool {
        self.get(key).is_ok()
    }
}

/// Map keyring errors to descriptive token store errors.
fn map_keyring_error(op: &str, account: &str, e: keyring::Error) -> TokenStoreError {
    match e {
        keyring::Error::NoEntry => TokenStoreError::NotFound(account.to_string()),
        keyring::Error::PlatformFailure(err) => {
            TokenStoreError::KeyringUnavailable(format!("failed to {} '{}': {}", op, account, err))
        }
        keyring::Error::NoStorageAccess(err) => TokenStoreError::KeyringUnavailable(format!(
            "failed to {} '{}' (no storage access): {}",
            op, account, err
        )),
        other => TokenStoreError::StoreFailed(format!("failed to {} '{}': {}", op, account, other)),
    }
}

/// Install keyring's in-process mock credential store (idempotent).
///
/// Test-only: keeps unit and integration tests away from the real
/// OS credential store (no macOS Keychain prompts, works headless).
/// `keyring::set_default_credential_builder` is process-global, so tests
/// touching the keyring must be serialized (`serial_test`).
#[cfg(debug_assertions)]
pub fn use_mock_keyring() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        keyring::set_default_credential_builder(keyring::mock::default_credential_builder());
    });
}

/// Helper function to create a token key from team_id and user_id
pub fn make_token_key(team_id: &str, user_id: &str) -> String {
    format!("{}:{}", team_id, user_id)
}

/// Helper function to create an OAuth client secret key for a profile
pub fn make_oauth_client_secret_key(profile_name: &str) -> String {
    format!("oauth-client-secret:{}", profile_name)
}

/// Store OAuth client secret in the token store
pub fn store_oauth_client_secret(
    token_store: &dyn TokenStore,
    profile_name: &str,
    client_secret: &str,
) -> Result<()> {
    let key = make_oauth_client_secret_key(profile_name);
    token_store.set(&key, client_secret)
}

/// Retrieve OAuth client secret from the token store
pub fn get_oauth_client_secret(token_store: &dyn TokenStore, profile_name: &str) -> Result<String> {
    let key = make_oauth_client_secret_key(profile_name);
    token_store.get(&key)
}

/// Delete OAuth client secret from the token store
pub fn delete_oauth_client_secret(token_store: &dyn TokenStore, profile_name: &str) -> Result<()> {
    let key = make_oauth_client_secret_key(profile_name);
    token_store.delete(&key)
}

/// Create the production token store (OS keyring).
///
/// Returns Box<dyn TokenStore> for runtime polymorphism.
pub fn create_token_store() -> Result<Box<dyn TokenStore>> {
    let store = KeyringTokenStore::new()?;
    Ok(Box::new(store))
}

/// Path where legacy plaintext tokens were stored before keyring-only storage.
///
/// Respects XDG_DATA_HOME when set; falls back to
/// `~/.local/share/slack-rs/tokens.json`. Only used by `auth migrate` and
/// the migration hint — never for reads or writes during normal operation.
pub fn legacy_tokens_path() -> Result<PathBuf> {
    if let Ok(xdg_data_home) = std::env::var("XDG_DATA_HOME") {
        let trimmed = xdg_data_home.trim();
        if !trimmed.is_empty() {
            let xdg_path = PathBuf::from(trimmed);
            if xdg_path.is_absolute() {
                return Ok(xdg_path.join("slack-rs").join("tokens.json"));
            }
        }
    }

    let home = directories::BaseDirs::new()
        .ok_or_else(|| TokenStoreError::IoError("Failed to determine home directory".to_string()))?
        .home_dir()
        .to_path_buf();

    Ok(home
        .join(".local")
        .join("share")
        .join("slack-rs")
        .join("tokens.json"))
}

/// Return a hint for users who still have a legacy plaintext tokens.json.
///
/// Appended to "no token found" errors so users know to run `auth migrate`.
pub fn legacy_tokens_hint() -> Option<String> {
    let path = legacy_tokens_path().ok()?;
    if path.exists() {
        Some(format!(
            "A legacy plaintext token file exists at {}. Run 'slack auth migrate' to move it into the OS keyring.",
            path.display()
        ))
    } else {
        None
    }
}

/// Summary of a legacy tokens.json migration.
#[derive(Debug)]
pub struct MigrationSummary {
    /// Number of secrets imported into the keyring
    pub imported: usize,
    /// Token store keys that were imported
    pub keys: Vec<String>,
    /// Path of the legacy file that was deleted
    pub source: PathBuf,
}

/// Migrate a legacy plaintext `tokens.json` into the given token store,
/// then securely delete the file (best-effort zero overwrite, then remove).
pub fn migrate_legacy_tokens(store: &dyn TokenStore, path: &Path) -> Result<MigrationSummary> {
    let content = std::fs::read_to_string(path).map_err(|e| {
        TokenStoreError::IoError(format!(
            "Failed to read legacy tokens file {}: {}",
            path.display(),
            e
        ))
    })?;

    let tokens: BTreeMap<String, String> = serde_json::from_str(&content).map_err(|e| {
        TokenStoreError::IoError(format!(
            "Failed to parse legacy tokens file {}: {}",
            path.display(),
            e
        ))
    })?;

    let mut keys = Vec::with_capacity(tokens.len());
    for (key, value) in &tokens {
        store.set(key, value)?;
        keys.push(key.clone());
    }

    shred_file(path)?;

    Ok(MigrationSummary {
        imported: keys.len(),
        keys,
        source: path.to_path_buf(),
    })
}

/// Best-effort secure deletion: overwrite contents with zeros, sync, remove.
fn shred_file(path: &Path) -> Result<()> {
    use std::io::{Seek, SeekFrom, Write};

    let len = std::fs::metadata(path)
        .map_err(|e| TokenStoreError::IoError(format!("Failed to stat {}: {}", path.display(), e)))?
        .len();

    // Overwrite with zeros (best-effort; ignore failure and still remove).
    if let Ok(mut file) = std::fs::OpenOptions::new().write(true).open(path) {
        let zeros = vec![0u8; len as usize];
        let _ = file.seek(SeekFrom::Start(0));
        let _ = file.write_all(&zeros);
        let _ = file.sync_all();
    }

    std::fs::remove_file(path).map_err(|e| {
        TokenStoreError::IoError(format!("Failed to remove {}: {}", path.display(), e))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    #[test]
    fn test_in_memory_token_store_set_get() {
        let store = InMemoryTokenStore::new();
        let key = "T123:U456";
        let token = "xoxb-test-token";

        store.set(key, token).unwrap();
        assert_eq!(store.get(key).unwrap(), token);
    }

    #[test]
    fn test_in_memory_token_store_delete() {
        let store = InMemoryTokenStore::new();
        let key = "T123:U456";
        let token = "xoxb-test-token";

        store.set(key, token).unwrap();
        assert!(store.exists(key));

        store.delete(key).unwrap();
        assert!(!store.exists(key));
        assert!(store.get(key).is_err());
    }

    #[test]
    fn test_in_memory_token_store_not_found() {
        let store = InMemoryTokenStore::new();
        let result = store.get("nonexistent");
        assert!(result.is_err());
        match result {
            Err(TokenStoreError::NotFound(_)) => {}
            _ => panic!("Expected NotFound error"),
        }
    }

    #[test]
    fn test_in_memory_token_store_exists() {
        let store = InMemoryTokenStore::new();
        let key = "T123:U456";

        assert!(!store.exists(key));
        store.set(key, "token").unwrap();
        assert!(store.exists(key));
    }

    #[test]
    fn test_make_token_key() {
        let key = make_token_key("T123", "U456");
        assert_eq!(key, "T123:U456");
    }

    #[test]
    fn test_in_memory_token_store_multiple_keys() {
        let store = InMemoryTokenStore::new();

        store.set("T1:U1", "token1").unwrap();
        store.set("T2:U2", "token2").unwrap();

        assert_eq!(store.get("T1:U1").unwrap(), "token1");
        assert_eq!(store.get("T2:U2").unwrap(), "token2");
    }

    #[test]
    fn test_make_oauth_client_secret_key() {
        let key = make_oauth_client_secret_key("default");
        assert_eq!(key, "oauth-client-secret:default");
    }

    #[test]
    fn test_store_and_get_oauth_client_secret() {
        let store = InMemoryTokenStore::new();
        let profile_name = "test-profile";
        let client_secret = "test-secret-123";

        store_oauth_client_secret(&store, profile_name, client_secret).unwrap();
        let retrieved = get_oauth_client_secret(&store, profile_name).unwrap();
        assert_eq!(retrieved, client_secret);
    }

    #[test]
    fn test_delete_oauth_client_secret() {
        let store = InMemoryTokenStore::new();
        let profile_name = "test-profile";
        let client_secret = "test-secret-123";

        store_oauth_client_secret(&store, profile_name, client_secret).unwrap();
        assert!(get_oauth_client_secret(&store, profile_name).is_ok());

        delete_oauth_client_secret(&store, profile_name).unwrap();
        assert!(get_oauth_client_secret(&store, profile_name).is_err());
    }

    #[test]
    fn test_account_grouping() {
        // Bot and user tokens for the same identity share an entry
        assert_eq!(KeyringTokenStore::account_for_key("T1:U1"), "T1:U1");
        assert_eq!(KeyringTokenStore::account_for_key("T1:U1:user"), "T1:U1");
        // Client secrets get their own entry
        assert_eq!(
            KeyringTokenStore::account_for_key("oauth-client-secret:default"),
            "oauth-client-secret:default"
        );
        // Arbitrary keys map to themselves
        assert_eq!(KeyringTokenStore::account_for_key("some-key"), "some-key");
    }

    /// Round-trip through the keyring store using keyring's mock backend.
    /// The mock builder is process-global, hence #[serial].
    #[test]
    #[serial]
    fn test_keyring_token_store_round_trip_via_mock() {
        use_mock_keyring();
        let store = KeyringTokenStore::new().unwrap();

        let bot_key = make_token_key("TKR1", "UKR1");
        let user_key = format!("{}:user", bot_key);
        let secret_key = make_oauth_client_secret_key("keyring-test");

        store.set(&bot_key, "xoxb-keyring-bot").unwrap();
        store.set(&user_key, "xoxp-keyring-user").unwrap();
        store.set(&secret_key, "keyring-secret").unwrap();

        assert_eq!(store.get(&bot_key).unwrap(), "xoxb-keyring-bot");
        assert_eq!(store.get(&user_key).unwrap(), "xoxp-keyring-user");
        assert_eq!(store.get(&secret_key).unwrap(), "keyring-secret");

        // Bot and user tokens share one keyring entry; deleting one
        // must not delete the other.
        store.delete(&bot_key).unwrap();
        assert!(!store.exists(&bot_key));
        assert!(store.exists(&user_key));

        store.delete(&user_key).unwrap();
        assert!(!store.exists(&user_key));

        store.delete(&secret_key).unwrap();
        assert!(!store.exists(&secret_key));
    }

    #[test]
    #[serial]
    fn test_keyring_token_store_not_found() {
        use_mock_keyring();
        let store = KeyringTokenStore::new().unwrap();
        match store.get("TNOPE:UNOPE") {
            Err(TokenStoreError::NotFound(_)) => {}
            other => panic!("Expected NotFound, got {:?}", other),
        }
    }

    #[test]
    #[serial]
    fn test_create_token_store_uses_keyring() {
        use_mock_keyring();
        let store = create_token_store().unwrap();
        store.set("TCTS:UCTS", "xoxb-cts").unwrap();
        assert_eq!(store.get("TCTS:UCTS").unwrap(), "xoxb-cts");
        store.delete("TCTS:UCTS").unwrap();
    }

    #[test]
    fn test_keyring_unavailable_message_is_actionable() {
        let err = TokenStoreError::KeyringUnavailable("boom".to_string());
        let msg = err.to_string();
        assert!(msg.contains("OS keyring unavailable"));
        assert!(msg.contains("Secret Service"));
        assert!(msg.contains("gnome-keyring"));
    }

    #[test]
    #[serial]
    fn test_migrate_legacy_tokens() {
        use tempfile::TempDir;
        use_mock_keyring();

        let temp_dir = TempDir::new().unwrap();
        let legacy_path = temp_dir.path().join("tokens.json");

        let mut legacy = BTreeMap::new();
        legacy.insert("TMIG:UMIG".to_string(), "xoxb-migrated".to_string());
        legacy.insert("TMIG:UMIG:user".to_string(), "xoxp-migrated".to_string());
        legacy.insert(
            "oauth-client-secret:default".to_string(),
            "migrated-secret".to_string(),
        );
        std::fs::write(&legacy_path, serde_json::to_string_pretty(&legacy).unwrap()).unwrap();

        let store = KeyringTokenStore::new().unwrap();
        let summary = migrate_legacy_tokens(&store, &legacy_path).unwrap();

        assert_eq!(summary.imported, 3);
        assert!(!legacy_path.exists(), "legacy file should be deleted");
        assert_eq!(store.get("TMIG:UMIG").unwrap(), "xoxb-migrated");
        assert_eq!(store.get("TMIG:UMIG:user").unwrap(), "xoxp-migrated");
        assert_eq!(
            store.get("oauth-client-secret:default").unwrap(),
            "migrated-secret"
        );

        // Cleanup mock store state for other tests
        store.delete("TMIG:UMIG").ok();
        store.delete("TMIG:UMIG:user").ok();
        store.delete("oauth-client-secret:default").ok();
    }

    #[test]
    fn test_migrate_legacy_tokens_missing_file() {
        use tempfile::TempDir;
        let temp_dir = TempDir::new().unwrap();
        let store = InMemoryTokenStore::new();
        let result = migrate_legacy_tokens(&store, &temp_dir.path().join("nope.json"));
        assert!(result.is_err());
    }

    #[test]
    fn test_migrate_legacy_tokens_into_in_memory_store() {
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();
        let legacy_path = temp_dir.path().join("tokens.json");
        std::fs::write(&legacy_path, r#"{"T1:U1":"xoxb-1"}"#).unwrap();

        let store = InMemoryTokenStore::new();
        let summary = migrate_legacy_tokens(&store, &legacy_path).unwrap();
        assert_eq!(summary.imported, 1);
        assert_eq!(summary.keys, vec!["T1:U1".to_string()]);
        assert_eq!(store.get("T1:U1").unwrap(), "xoxb-1");
        assert!(!legacy_path.exists());
    }

    #[test]
    #[serial]
    fn test_legacy_tokens_path_respects_xdg() {
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();
        std::env::set_var("XDG_DATA_HOME", temp_dir.path().to_str().unwrap());
        let path = legacy_tokens_path().unwrap();
        assert_eq!(path, temp_dir.path().join("slack-rs").join("tokens.json"));
        std::env::remove_var("XDG_DATA_HOME");
    }

    #[test]
    #[serial]
    fn test_legacy_tokens_hint_when_file_exists() {
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();
        std::env::set_var("XDG_DATA_HOME", temp_dir.path().to_str().unwrap());

        // No file yet -> no hint
        assert!(legacy_tokens_hint().is_none());

        let dir = temp_dir.path().join("slack-rs");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("tokens.json"), "{}").unwrap();

        let hint = legacy_tokens_hint().expect("hint should be present");
        assert!(hint.contains("slack auth migrate"));

        std::env::remove_var("XDG_DATA_HOME");
    }

    /// InMemoryTokenStore works as a test/mock backend with the same
    /// key formats as the production keyring backend.
    #[test]
    fn test_in_memory_token_store_as_mock() {
        let store = InMemoryTokenStore::new();

        let token_key = make_token_key("T999", "U888");
        store.set(&token_key, "xoxb-mock-token").unwrap();
        assert_eq!(store.get(&token_key).unwrap(), "xoxb-mock-token");

        let secret_key = make_oauth_client_secret_key("mock-profile");
        store.set(&secret_key, "mock-secret").unwrap();
        assert_eq!(store.get(&secret_key).unwrap(), "mock-secret");

        assert!(store.exists(&token_key));
        assert!(store.exists(&secret_key));
        assert!(!store.exists("nonexistent"));

        store.delete(&token_key).unwrap();
        assert!(!store.exists(&token_key));

        store_oauth_client_secret(&store, "test", "test-secret").unwrap();
        assert_eq!(
            get_oauth_client_secret(&store, "test").unwrap(),
            "test-secret"
        );
        delete_oauth_client_secret(&store, "test").unwrap();
        assert!(!store.exists(&make_oauth_client_secret_key("test")));
    }
}
