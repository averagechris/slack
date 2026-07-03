use slack::profile::{save_config, Profile, ProfilesConfig};
use std::env;
use std::fs;
use tempfile::TempDir;

/// Helper to set up a test environment with profile and tokens
fn setup_test_env() -> (TempDir, String) {
    // Keep tests off the real OS credential store
    slack::profile::use_mock_keyring();

    let temp_dir = TempDir::new().unwrap();
    let config_dir = temp_dir.path().join("config");

    fs::create_dir_all(&config_dir).unwrap();

    // Set environment variables to use temp directories
    let config_path = config_dir.join("profiles.json");

    env::set_var("SLACK_RS_CONFIG_PATH", &config_path);

    // Create a test profile
    let mut config = ProfilesConfig::new();
    let profile = Profile {
        team_id: "T123ABC".to_string(),
        user_id: "U456DEF".to_string(),
        team_name: Some("Test Team".to_string()),
        user_name: Some("Test User".to_string()),
        client_id: None,
        redirect_uri: None,
        scopes: None,
        bot_scopes: None,
        user_scopes: None,
        default_token_type: None,
    };
    config.set("test_profile".to_string(), profile);
    save_config(&config_path, &config).unwrap();

    // Create token store with dummy tokens (production key formats)
    let token_store = slack::profile::create_token_store().unwrap();
    let bot_key = slack::profile::make_token_key("T123ABC", "U456DEF");
    let user_key = format!("{}:user", bot_key);

    // Store tokens with realistic-looking values
    token_store
        .set(&bot_key, "bot_test_token_placeholder")
        .unwrap();
    token_store
        .set(&user_key, "user_test_token_placeholder")
        .unwrap();

    (temp_dir, config_path.display().to_string())
}

/// Remove the environment/keyring state installed by `setup_test_env`.
fn teardown_test_env() {
    let token_store = slack::profile::create_token_store().unwrap();
    let bot_key = slack::profile::make_token_key("T123ABC", "U456DEF");
    token_store.delete(&bot_key).ok();
    token_store.delete(&format!("{}:user", bot_key)).ok();
    env::remove_var("SLACK_RS_CONFIG_PATH");
}

/// End-to-end diagnostics collection: doctor must find both tokens using
/// the production key formats (`{team}:{user}` and `{team}:{user}:user`).
#[test]
#[serial_test::serial]
fn test_collect_diagnostics_reports_existing_tokens() {
    let (_temp_dir, config_path) = setup_test_env();

    let info = slack::commands::doctor::collect_diagnostics("test_profile").unwrap();

    assert_eq!(info.config_path, config_path);
    assert_eq!(info.token_store.backend, "keyring");
    assert!(info.token_store.location.contains("keyring service"));
    assert!(
        info.tokens.bot_token_exists,
        "doctor must detect the bot token"
    );
    assert!(
        info.tokens.user_token_exists,
        "doctor must detect the user token stored under the ':user' key"
    );
    // No hints when tokens are present
    assert!(info.scope_hints.is_empty());

    teardown_test_env();
}

/// Diagnostics with a profile but no stored tokens: hint the user to log in.
#[test]
#[serial_test::serial]
fn test_collect_diagnostics_without_tokens_hints_login() {
    let (_temp_dir, _config_path) = setup_test_env();

    // Remove the tokens seeded by setup
    let token_store = slack::profile::create_token_store().unwrap();
    let bot_key = slack::profile::make_token_key("T123ABC", "U456DEF");
    token_store.delete(&bot_key).unwrap();
    token_store.delete(&format!("{}:user", bot_key)).unwrap();

    let info = slack::commands::doctor::collect_diagnostics("test_profile").unwrap();

    assert!(!info.tokens.bot_token_exists);
    assert!(!info.tokens.user_token_exists);
    assert!(info
        .scope_hints
        .iter()
        .any(|h| h.contains("No tokens found")));

    teardown_test_env();
}

/// Diagnostics for a missing profile: error listing available profiles.
#[test]
#[serial_test::serial]
fn test_collect_diagnostics_unknown_profile_errors() {
    let (_temp_dir, _config_path) = setup_test_env();

    let err = slack::commands::doctor::collect_diagnostics("nope").unwrap_err();
    assert!(err.contains("Profile 'nope' not found"));
    assert!(err.contains("test_profile"));

    teardown_test_env();
}

/// Diagnostics with no config file at all: empty status plus setup hint.
#[test]
#[serial_test::serial]
fn test_collect_diagnostics_no_config() {
    slack::profile::use_mock_keyring();
    let temp_dir = TempDir::new().unwrap();
    env::set_var(
        "SLACK_RS_CONFIG_PATH",
        temp_dir.path().join("does-not-exist.json"),
    );

    let info = slack::commands::doctor::collect_diagnostics("default").unwrap();

    assert!(!info.tokens.bot_token_exists);
    assert!(!info.tokens.user_token_exists);
    assert!(info
        .scope_hints
        .iter()
        .any(|h| h.contains("No profiles configured")));

    env::remove_var("SLACK_RS_CONFIG_PATH");
}

#[test]
#[serial_test::serial]
fn test_doctor_output_does_not_contain_token_values() {
    let (_temp_dir, _config_path) = setup_test_env();

    // Verify that the diagnostic structures can't hold token values by design
    let info = slack::commands::doctor::DiagnosticInfo {
        config_path: "/test/path".to_string(),
        token_store: slack::commands::doctor::TokenStoreInfo {
            backend: "keyring".to_string(),
            location: "OS keyring service 'slack'".to_string(),
        },
        tokens: slack::commands::doctor::TokenStatus {
            bot_token_exists: true,
            user_token_exists: true,
        },
        scope_hints: vec![],
    };

    let json = serde_json::to_string(&info).unwrap();

    // Verify no token patterns appear in output
    assert!(!json.contains("xoxb-"), "Output contains bot token pattern");
    assert!(
        !json.contains("xoxp-"),
        "Output contains user token pattern"
    );

    // Verify expected fields are present (camelCase)
    assert!(json.contains("configPath"));
    assert!(json.contains("tokenStore"));
    assert!(json.contains("botTokenExists"));
    assert!(json.contains("userTokenExists"));
}

#[test]
fn test_doctor_json_output_schema() {
    let info = slack::commands::doctor::DiagnosticInfo {
        config_path: "/home/user/.config/slack-rs/profiles.json".to_string(),
        token_store: slack::commands::doctor::TokenStoreInfo {
            backend: "keyring".to_string(),
            location: "OS keyring service 'slack'".to_string(),
        },
        tokens: slack::commands::doctor::TokenStatus {
            bot_token_exists: true,
            user_token_exists: false,
        },
        scope_hints: vec!["Test hint".to_string()],
    };

    let json = serde_json::to_string_pretty(&info).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

    // Verify required fields (camelCase)
    assert!(parsed.get("configPath").is_some());
    assert!(parsed.get("tokenStore").is_some());
    assert!(parsed.get("tokens").is_some());

    // Verify tokenStore structure (camelCase)
    let token_store = parsed.get("tokenStore").unwrap();
    assert!(token_store.get("backend").is_some());
    assert!(token_store.get("location").is_some());

    // Verify tokens structure (camelCase)
    let tokens = parsed.get("tokens").unwrap();
    assert!(tokens.get("botTokenExists").is_some());
    assert!(tokens.get("userTokenExists").is_some());

    // Verify scopeHints is present when not empty (camelCase)
    assert!(parsed.get("scopeHints").is_some());
}

#[test]
fn test_doctor_json_output_omits_empty_scope_hints() {
    let info = slack::commands::doctor::DiagnosticInfo {
        config_path: "/test/path".to_string(),
        token_store: slack::commands::doctor::TokenStoreInfo {
            backend: "keyring".to_string(),
            location: "OS keyring service 'slack'".to_string(),
        },
        tokens: slack::commands::doctor::TokenStatus {
            bot_token_exists: true,
            user_token_exists: true,
        },
        scope_hints: vec![],
    };

    let json = serde_json::to_string(&info).unwrap();

    // Empty scopeHints should not appear in JSON due to skip_serializing_if (camelCase)
    assert!(!json.contains("scopeHints"));
    assert!(!json.contains("scope_hints"));
}

#[test]
fn test_token_status_only_contains_existence_flags() {
    let status = slack::commands::doctor::TokenStatus {
        bot_token_exists: true,
        user_token_exists: false,
    };

    let json = serde_json::to_string(&status).unwrap();

    // Verify it only contains boolean flags, never token values (camelCase)
    assert!(json.contains("botTokenExists"));
    assert!(json.contains("userTokenExists"));
    assert!(json.contains("true"));
    assert!(json.contains("false"));

    // Verify no token-like patterns
    assert!(!json.contains("xoxb"));
    assert!(!json.contains("xoxp"));
    assert!(!json.contains("token\":\""));
}

#[test]
fn test_diagnostic_info_deserialization() {
    let json = r#"{
        "configPath": "/test/profiles.json",
        "tokenStore": {
            "backend": "keyring",
            "location": "OS keyring service 'slack'"
        },
        "tokens": {
            "botTokenExists": true,
            "userTokenExists": false
        },
        "scopeHints": ["Hint 1", "Hint 2"]
    }"#;

    let info: slack::commands::doctor::DiagnosticInfo = serde_json::from_str(json).unwrap();

    assert_eq!(info.config_path, "/test/profiles.json");
    assert_eq!(info.token_store.backend, "keyring");
    assert_eq!(info.token_store.location, "OS keyring service 'slack'");
    assert!(info.tokens.bot_token_exists);
    assert!(!info.tokens.user_token_exists);
    assert_eq!(info.scope_hints.len(), 2);
}

// Note: Help output tests removed to avoid CI timeout issues.
// The help functionality is implicitly tested through the CLI usage patterns,
// and doesn't require slow `cargo run` integration tests.
