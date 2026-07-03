//! CLI dispatch integration tests
//!
//! Exercises command execution end-to-end through the clap layer: argv is
//! parsed with the real `Cli` clap model (exactly what `main.rs` does) and
//! then dispatched to the same `cli::run_*` handlers, against mocked HTTP
//! servers and the mock keyring. No network, no real credential store.
//!
//! In debug builds the API client honors `SLACK_API_BASE_URL` so both
//! `api call` and the wrapper commands can be pointed at an httpmock server.

use clap::Parser;
use httpmock::prelude::*;
use serde_json::json;
use serial_test::serial;
use slack::cli::args::{ApiCommand, AuthCommand, Cli, Command, MsgCommand};
use slack::profile::{save_config, Profile, ProfilesConfig};
use tempfile::TempDir;

const TEAM_ID: &str = "TDSP";
const USER_ID: &str = "UDSP";

/// Parse argv through the real clap model, as main() does.
fn parse(argv: &[&str]) -> Cli {
    Cli::try_parse_from(argv).expect("argv should parse")
}

fn globals() -> slack::cli::GlobalArgs {
    slack::cli::GlobalArgs {
        profile: None,
        non_interactive: true,
        debug_level: slack::debug::DebugLevel::Off,
    }
}

/// Set up an isolated profile config + mock keyring for a test.
///
/// Returns the TempDir guard; SLACK_RS_CONFIG_PATH points inside it.
fn setup_profile() -> TempDir {
    slack::profile::use_mock_keyring();
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("profiles.json");

    let mut config = ProfilesConfig::new();
    config.set(
        "default".to_string(),
        Profile {
            team_id: TEAM_ID.to_string(),
            user_id: USER_ID.to_string(),
            team_name: Some("Dispatch Team".to_string()),
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
    std::env::set_var("SLACK_RS_CONFIG_PATH", &config_path);
    temp_dir
}

fn seed_bot_token(token: &str) {
    let store = slack::profile::create_token_store().unwrap();
    store
        .set(&slack::profile::make_token_key(TEAM_ID, USER_ID), token)
        .unwrap();
}

/// Remove per-test global state (env vars + mock keyring entries).
fn teardown() {
    let store = slack::profile::create_token_store().unwrap();
    let bot_key = slack::profile::make_token_key(TEAM_ID, USER_ID);
    store.delete(&bot_key).ok();
    store.delete(&format!("{}:user", bot_key)).ok();
    std::env::remove_var("SLACK_RS_CONFIG_PATH");
    std::env::remove_var("SLACK_API_BASE_URL");
    std::env::remove_var("SLACKCLI_ALLOW_WRITE");
    std::env::remove_var("XDG_DATA_HOME");
}

/// Extract the free-form `rest` args from a parsed `api call` command.
fn api_call_rest(cli: Cli) -> Vec<String> {
    match cli.command {
        Command::Api {
            command: ApiCommand::Call(call_args),
        } => call_args.rest,
        other => panic!("expected api call, got {:?}", other),
    }
}

/// `api call` end-to-end: clap parse -> dispatch -> mocked Slack endpoint,
/// with the stored bot token sent as a bearer token.
#[tokio::test]
#[serial]
async fn test_api_call_dispatch_hits_mock_server_with_bearer_token() {
    let _temp = setup_profile();
    seed_bot_token("xoxb-dispatch-token");

    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(POST)
            .path("/auth.test")
            .header("authorization", "Bearer xoxb-dispatch-token");
        then.status(200).json_body(json!({
            "ok": true,
            "team_id": TEAM_ID,
            "user_id": USER_ID
        }));
    });
    std::env::set_var("SLACK_API_BASE_URL", server.base_url());

    let cli = parse(&["slack", "api", "call", "auth.test"]);
    let rest = api_call_rest(cli);
    let result = slack::cli::run_api_call(rest).await;

    assert!(result.is_ok(), "api call failed: {:?}", result.err());
    mock.assert();

    teardown();
}

/// `api call` with key=value params forwards them as form data.
#[tokio::test]
#[serial]
async fn test_api_call_dispatch_forwards_params() {
    let _temp = setup_profile();
    seed_bot_token("xoxb-dispatch-token");

    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(POST)
            .path("/conversations.info")
            .form_urlencoded_tuple("channel", "C123");
        then.status(200)
            .json_body(json!({"ok": true, "channel": {"id": "C123"}}));
    });
    std::env::set_var("SLACK_API_BASE_URL", server.base_url());

    let cli = parse(&["slack", "api", "call", "conversations.info", "channel=C123"]);
    let result = slack::cli::run_api_call(api_call_rest(cli)).await;

    assert!(result.is_ok(), "api call failed: {:?}", result.err());
    mock.assert();

    teardown();
}

/// `api call` with no stored token fails with a login hint and never
/// touches the server.
#[tokio::test]
#[serial]
async fn test_api_call_dispatch_without_token_errors() {
    let _temp = setup_profile();

    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.any_request();
        then.status(200).json_body(json!({"ok": true}));
    });
    std::env::set_var("SLACK_API_BASE_URL", server.base_url());

    let cli = parse(&["slack", "api", "call", "auth.test"]);
    let result = slack::cli::run_api_call(api_call_rest(cli)).await;

    let err = result.expect_err("must fail without a token").to_string();
    assert!(err.contains("No bot token found"), "got: {}", err);
    assert!(err.contains("slack auth login"), "got: {}", err);
    assert_eq!(mock.calls(), 0, "no request may be sent without a token");

    teardown();
}

/// When an unmigrated legacy tokens.json exists, token errors must point
/// the user at `slack auth migrate`.
#[tokio::test]
#[serial]
async fn test_api_call_token_error_includes_migrate_hint_when_legacy_file_exists() {
    let _temp = setup_profile();

    // Plant a legacy tokens.json under a temp XDG_DATA_HOME
    let data_dir = TempDir::new().unwrap();
    std::env::set_var("XDG_DATA_HOME", data_dir.path());
    let legacy_dir = data_dir.path().join("slack-rs");
    std::fs::create_dir_all(&legacy_dir).unwrap();
    std::fs::write(legacy_dir.join("tokens.json"), "{}").unwrap();

    let cli = parse(&["slack", "api", "call", "auth.test"]);
    let result = slack::cli::run_api_call(api_call_rest(cli)).await;

    let err = result.expect_err("must fail without a token").to_string();
    assert!(err.contains("slack auth migrate"), "got: {}", err);

    teardown();
}

/// `msg post` (write command) blocked by SLACKCLI_ALLOW_WRITE=false:
/// the guard must fire before any request is sent.
#[tokio::test]
#[serial]
async fn test_msg_post_dispatch_blocked_by_write_guard() {
    let _temp = setup_profile();
    seed_bot_token("xoxb-dispatch-token");

    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.any_request();
        then.status(200).json_body(json!({"ok": true}));
    });
    std::env::set_var("SLACK_API_BASE_URL", server.base_url());
    std::env::set_var("SLACKCLI_ALLOW_WRITE", "false");

    let cli = parse(&["slack", "msg", "post", "C123", "hello", "--yes"]);
    let post_args = match cli.command {
        Command::Msg {
            command: MsgCommand::Post(post_args),
        } => post_args,
        other => panic!("expected msg post, got {:?}", other),
    };

    let result = slack::cli::run_msg_post(&post_args, &globals()).await;

    let err = result.expect_err("write must be blocked");
    assert!(err.contains("Write operation denied"), "got: {}", err);
    assert_eq!(mock.calls(), 0, "guard must block before any request");

    teardown();
}

/// `msg post` allowed: guard passes, chat.postMessage is called on the
/// mocked server with the right payload.
#[tokio::test]
#[serial]
async fn test_msg_post_dispatch_allowed_posts_message() {
    let _temp = setup_profile();
    seed_bot_token("xoxb-dispatch-token");

    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(POST)
            .path("/chat.postMessage")
            .header("authorization", "Bearer xoxb-dispatch-token")
            .json_body_includes(r#"{"channel": "C123", "text": "hello world"}"#);
        then.status(200).json_body(json!({
            "ok": true,
            "channel": "C123",
            "ts": "1234567890.123456"
        }));
    });
    std::env::set_var("SLACK_API_BASE_URL", server.base_url());
    std::env::set_var("SLACKCLI_ALLOW_WRITE", "true");

    let cli = parse(&["slack", "msg", "post", "C123", "hello world", "--yes"]);
    let post_args = match cli.command {
        Command::Msg {
            command: MsgCommand::Post(post_args),
        } => post_args,
        other => panic!("expected msg post, got {:?}", other),
    };

    let result = slack::cli::run_msg_post(&post_args, &globals()).await;

    assert!(result.is_ok(), "msg post failed: {:?}", result.err());
    mock.assert();

    teardown();
}

/// `auth migrate` end-to-end through the clap layer: legacy tokens land in
/// the (mock) keyring and the plaintext file is deleted.
#[tokio::test]
#[serial]
async fn test_auth_migrate_dispatch_imports_and_shreds_legacy_file() {
    let _temp = setup_profile();

    let legacy_dir = TempDir::new().unwrap();
    let legacy_path = legacy_dir.path().join("tokens.json");
    std::fs::write(
        &legacy_path,
        format!(r#"{{"{}:{}":"xoxb-migrated-token"}}"#, TEAM_ID, USER_ID),
    )
    .unwrap();

    let cli = parse(&[
        "slack",
        "auth",
        "migrate",
        "--path",
        legacy_path.to_str().unwrap(),
    ]);
    let path = match cli.command {
        Command::Auth {
            command: AuthCommand::Migrate { path },
        } => path,
        other => panic!("expected auth migrate, got {:?}", other),
    };

    let result = slack::auth::migrate(path);

    assert!(result.is_ok(), "migrate failed: {:?}", result.err());
    assert!(!legacy_path.exists(), "legacy file must be deleted");
    let store = slack::profile::create_token_store().unwrap();
    assert_eq!(
        store
            .get(&slack::profile::make_token_key(TEAM_ID, USER_ID))
            .unwrap(),
        "xoxb-migrated-token"
    );

    teardown();
}

/// `auth status` dispatch: succeeds with and without stored tokens (the
/// report contents are asserted in the auth unit tests).
#[tokio::test]
#[serial]
async fn test_auth_status_dispatch_with_and_without_tokens() {
    let _temp = setup_profile();

    // Without tokens
    let cli = parse(&["slack", "auth", "status"]);
    let profile_name = match cli.command {
        Command::Auth {
            command: AuthCommand::Status { profile_name },
        } => profile_name,
        other => panic!("expected auth status, got {:?}", other),
    };
    assert!(slack::auth::status(profile_name.clone()).is_ok());

    // With tokens
    seed_bot_token("xoxb-dispatch-token");
    assert!(slack::auth::status(profile_name).is_ok());

    // Unknown profile errors
    let err = slack::auth::status(Some("missing".to_string())).unwrap_err();
    assert!(err.contains("not found"));

    teardown();
}

/// `doctor` dispatch: both text and JSON modes succeed against the mock
/// keyring (output shape is asserted via `collect_diagnostics` tests).
#[tokio::test]
#[serial]
async fn test_doctor_dispatch_text_and_json() {
    let _temp = setup_profile();
    seed_bot_token("xoxb-dispatch-token");

    let cli = parse(&["slack", "doctor", "--json"]);
    let doctor_args = match cli.command {
        Command::Doctor(doctor_args) => doctor_args,
        other => panic!("expected doctor, got {:?}", other),
    };
    assert!(doctor_args.json);
    assert!(slack::commands::doctor(None, doctor_args.json).is_ok());
    // Text mode
    assert!(slack::commands::doctor(None, false).is_ok());

    // The underlying diagnostics reflect the seeded bot token
    let info = slack::commands::doctor::collect_diagnostics("default").unwrap();
    assert!(info.tokens.bot_token_exists);
    assert!(!info.tokens.user_token_exists);

    teardown();
}
