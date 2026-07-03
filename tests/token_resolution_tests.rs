//! Integration tests for token resolution and response metadata
//!
//! Token storage is keyring-only; these tests use the keyring mock
//! credential store (debug-only escape hatch) so they never touch the
//! real OS credential store and pass headless.

use serde_json::json;
use slack::cli::get_api_client_with_token_type;
use slack::profile::{save_config, Profile, ProfilesConfig};
use std::env;
use tempfile::TempDir;

/// Setup test environment with a profile but no tokens in the (mock) keyring
fn setup_test_profile() -> (TempDir, String) {
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("profiles.json");

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
    config.set("default".to_string(), profile);
    save_config(&config_path, &config).unwrap();

    (temp_dir, config_path.to_string_lossy().to_string())
}

#[tokio::test]
#[serial_test::serial]
async fn test_token_resolution_uses_keyring_store() {
    // Keep the test off the real OS credential store
    slack::profile::use_mock_keyring();

    let (_temp_dir, config_path) = setup_test_profile();
    env::set_var("SLACK_RS_CONFIG_PATH", &config_path);

    // Store a bot token via the production store factory (mock-backed)
    let token_store = slack::profile::create_token_store().unwrap();
    let bot_key = slack::profile::make_token_key("T123ABC", "U456DEF");
    token_store.set(&bot_key, "xoxb-keyring-token").unwrap();

    let client = get_api_client_with_token_type(Some("default".to_string()), None).await;

    // Cleanup
    token_store.delete(&bot_key).ok();
    env::remove_var("SLACK_RS_CONFIG_PATH");

    assert!(
        client.is_ok(),
        "Should resolve token from the keyring store: {:?}",
        client.err()
    );
}

#[tokio::test]
#[serial_test::serial]
async fn test_token_resolution_fails_without_tokens() {
    slack::profile::use_mock_keyring();

    let (_temp_dir, config_path) = setup_test_profile();
    env::set_var("SLACK_RS_CONFIG_PATH", &config_path);

    // No tokens in the store: resolution must fail with a helpful error
    let client_result = get_api_client_with_token_type(Some("default".to_string()), None).await;

    env::remove_var("SLACK_RS_CONFIG_PATH");

    assert!(
        client_result.is_err(),
        "Should fail when no tokens exist in the store"
    );
}

#[tokio::test]
#[serial_test::serial]
async fn test_explicit_token_type_not_available_fails() {
    slack::profile::use_mock_keyring();

    let (_temp_dir, config_path) = setup_test_profile();
    env::set_var("SLACK_RS_CONFIG_PATH", &config_path);

    // Only a bot token exists; explicitly requesting a user token must fail
    let token_store = slack::profile::create_token_store().unwrap();
    let bot_key = slack::profile::make_token_key("T123ABC", "U456DEF");
    token_store.set(&bot_key, "xoxb-keyring-token").unwrap();

    let client_result = get_api_client_with_token_type(
        Some("default".to_string()),
        Some(slack::profile::TokenType::User),
    )
    .await;

    token_store.delete(&bot_key).ok();
    env::remove_var("SLACK_RS_CONFIG_PATH");

    assert!(
        client_result.is_err(),
        "Explicitly requested user token should not fall back to bot token"
    );
}

#[test]
fn test_command_response_with_token_type_metadata() {
    use slack::api::CommandResponse;

    // Test that CommandResponse::with_token_type includes token_type in metadata
    let response = CommandResponse::with_token_type(
        json!({"ok": true, "channels": []}),
        Some("default".to_string()),
        "T123ABC".to_string(),
        "U456DEF".to_string(),
        "conversations.list".to_string(),
        "conv list".to_string(),
        Some("bot".to_string()),
    );

    let json = serde_json::to_value(&response).unwrap();
    assert_eq!(json["meta"]["token_type"], "bot");
}

#[test]
fn test_command_response_with_user_token_type_metadata() {
    use slack::api::CommandResponse;

    // Test that CommandResponse::with_token_type works with user token type
    let response = CommandResponse::with_token_type(
        json!({"ok": true}),
        Some("default".to_string()),
        "T123".to_string(),
        "U456".to_string(),
        "users.info".to_string(),
        "users info".to_string(),
        Some("user".to_string()),
    );

    let json = serde_json::to_value(&response).unwrap();
    assert_eq!(json["meta"]["token_type"], "user");
}

#[test]
fn test_command_response_without_token_type_metadata() {
    use slack::api::CommandResponse;

    // Test that CommandResponse::with_token_type with None doesn't include token_type
    let response = CommandResponse::with_token_type(
        json!({"ok": true}),
        Some("default".to_string()),
        "T123".to_string(),
        "U456".to_string(),
        "users.info".to_string(),
        "users info".to_string(),
        None,
    );

    let json_str = serde_json::to_string(&response).unwrap();
    // token_type should not be present in JSON when None
    assert!(!json_str.contains("token_type"));
}
