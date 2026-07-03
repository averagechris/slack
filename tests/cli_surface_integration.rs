//! CLI surface integration tests
//!
//! Asserts the clap-based CLI preserves the pre-clap command surface:
//! commands parse, key flags are accepted, exit codes follow the contract
//! (0 success, 1 error/usage error, 2 non-interactive-input-required), and
//! the introspection commands emit usable JSON.

use std::process::Command;

fn run(args: &[&str]) -> (i32, String, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_slack"))
        .args(args)
        .env("SLACK_KEYRING_MOCK", "1")
        .output()
        .expect("Failed to execute slack binary");
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
    )
}

#[test]
fn test_help_exits_zero() {
    let (code, stdout, _) = run(&["--help"]);
    assert_eq!(code, 0);
    assert!(stdout.contains("Usage: slack"));
    for cmd in [
        "api",
        "auth",
        "config",
        "search",
        "conv",
        "thread",
        "users",
        "msg",
        "react",
        "file",
        "commands",
        "schema",
        "doctor",
        "install-skills",
        "completions",
    ] {
        assert!(stdout.contains(cmd), "top-level help should list '{}'", cmd);
    }
}

#[test]
fn test_version_flags() {
    let (code, stdout, _) = run(&["--version"]);
    assert_eq!(code, 0);
    assert!(stdout.starts_with("slack "));

    let (code, stdout, _) = run(&["-v"]);
    assert_eq!(code, 0);
    assert!(stdout.starts_with("slack "));
}

#[test]
fn test_no_args_shows_help_and_exits_zero() {
    let (code, _, _) = run(&[]);
    assert_eq!(code, 0);
}

#[test]
fn test_unknown_command_exits_one() {
    let (code, _, stderr) = run(&["definitely-not-a-command"]);
    assert_eq!(code, 1);
    assert!(stderr.contains("unrecognized subcommand"));
}

#[test]
fn test_unknown_flag_exits_one() {
    let (code, _, _) = run(&["auth", "login", "--ngrok"]);
    assert_eq!(code, 1);
    let (code, _, _) = run(&["conv", "list", "--bogus-flag"]);
    assert_eq!(code, 1);
}

#[test]
fn test_missing_required_args_exit_one() {
    for args in [
        vec!["search"],
        vec!["conv", "search"],
        vec!["thread", "get", "C123"],
        vec!["msg", "post", "C123"],
        vec!["msg", "update", "C123", "1.2"],
        vec!["react", "add", "C123", "1.2"],
        vec!["auth", "rename", "only-one"],
        vec!["users", "info"],
        vec!["users", "resolve-mentions"],
        vec!["file", "upload"],
    ] {
        let (code, _, _) = run(&args);
        assert_eq!(code, 1, "expected exit 1 for {:?}", args);
    }
}

#[test]
fn test_subcommand_help_exits_zero() {
    for args in [
        vec!["api", "call", "--help"],
        vec!["auth", "login", "--help"],
        vec!["auth", "export", "--help"],
        vec!["auth", "import", "--help"],
        vec!["auth", "migrate", "--help"],
        vec!["config", "oauth", "set", "--help"],
        vec!["conv", "list", "--help"],
        vec!["conv", "history", "--help"],
        vec!["thread", "get", "--help"],
        vec!["users", "cache-update", "--help"],
        vec!["msg", "post", "--help"],
        vec!["react", "add", "--help"],
        vec!["file", "download", "--help"],
        vec!["doctor", "--help"],
        vec!["install-skills", "--help"],
    ] {
        let (code, _, _) = run(&args);
        assert_eq!(code, 0, "expected exit 0 for {:?}", args);
    }
}

#[test]
fn test_commands_json_surface() {
    let (code, stdout, _) = run(&["commands", "--json"]);
    assert_eq!(code, 0);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    assert_eq!(parsed["schemaVersion"], 1);
    assert_eq!(parsed["type"], "commands.list");
    assert_eq!(parsed["ok"], true);

    let names: Vec<String> = parsed["commands"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap().to_string())
        .collect();

    // Every command from the pre-clap surface must still be present
    for expected in [
        "api call",
        "auth login",
        "auth status",
        "auth list",
        "auth rename",
        "auth logout",
        "auth export",
        "auth import",
        "auth migrate",
        "config oauth set",
        "config oauth show",
        "config oauth delete",
        "config set",
        "search",
        "conv list",
        "conv select",
        "conv search",
        "conv history",
        "thread get",
        "users info",
        "users cache-update",
        "users resolve-mentions",
        "msg post",
        "msg update",
        "msg delete",
        "react add",
        "react remove",
        "file upload",
        "file download",
        "commands",
        "schema",
        "doctor",
        "install-skills",
        "completions",
    ] {
        assert!(
            names.contains(&expected.to_string()),
            "commands --json missing '{}' (got: {:?})",
            expected,
            names
        );
    }
}

#[test]
fn test_commands_json_key_flags_present() {
    let (code, stdout, _) = run(&["commands", "--json"]);
    assert_eq!(code, 0);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let commands = parsed["commands"].as_array().unwrap();

    let flags_of = |name: &str| -> Vec<String> {
        commands
            .iter()
            .find(|c| c["name"] == name)
            .unwrap_or_else(|| panic!("command '{}' not found", name))["flags"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["name"].as_str().unwrap().to_string())
            .collect()
    };

    for flag in ["--thread-ts", "--reply-broadcast", "--idempotency-key"] {
        assert!(flags_of("msg post").contains(&flag.to_string()));
    }
    for flag in [
        "--types",
        "--include-private",
        "--all",
        "--limit",
        "--filter",
        "--format",
        "--sort",
        "--sort-dir",
        "--raw",
        "--token-type",
        "--profile",
    ] {
        assert!(flags_of("conv list").contains(&flag.to_string()));
    }
    for flag in [
        "--client-id",
        "--bot-scopes",
        "--user-scopes",
        "--cloudflared",
    ] {
        assert!(flags_of("auth login").contains(&flag.to_string()));
    }
    for flag in ["--out", "--passphrase-env", "--yes", "--lang", "--all"] {
        assert!(flags_of("auth export").contains(&flag.to_string()));
    }
    for flag in ["--in", "--force", "--dry-run", "--json"] {
        assert!(flags_of("auth import").contains(&flag.to_string()));
    }
    for flag in [
        "--client-id",
        "--redirect-uri",
        "--scopes",
        "--client-secret-env",
        "--client-secret-file",
    ] {
        assert!(flags_of("config oauth set").contains(&flag.to_string()));
    }
    assert!(flags_of("auth migrate").contains(&"--path".to_string()));
    assert!(flags_of("file download").contains(&"--url".to_string()));
    assert!(flags_of("file download").contains(&"--out".to_string()));
    for flag in ["--count", "--page", "--sort", "--sort_dir"] {
        assert!(flags_of("search").contains(&flag.to_string()));
    }
}

#[test]
fn test_commands_without_json_flag_exits_one() {
    let (code, _, stderr) = run(&["commands"]);
    assert_eq!(code, 1);
    assert!(stderr.contains("commands --json"));
}

#[test]
fn test_schema_command() {
    let (code, stdout, _) = run(&[
        "schema",
        "--command",
        "conv list",
        "--output",
        "json-schema",
    ]);
    assert_eq!(code, 0);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(parsed["type"], "schema");
    assert_eq!(parsed["command"], "conv list");

    // Dot-separated lookup still works
    let (code, _, _) = run(&[
        "schema",
        "--command",
        "conv.list",
        "--output",
        "json-schema",
    ]);
    assert_eq!(code, 0);

    // Missing flags -> usage error, exit 1
    let (code, _, _) = run(&["schema"]);
    assert_eq!(code, 1);

    // Invalid output format -> exit 1
    let (code, _, _) = run(&["schema", "--command", "conv list", "--output", "yaml"]);
    assert_eq!(code, 1);

    // Unknown command -> exit 1
    let (code, _, _) = run(&["schema", "--command", "nope", "--output", "json-schema"]);
    assert_eq!(code, 1);
}

#[test]
fn test_help_json_top_level() {
    let (code, stdout, _) = run(&["--help", "--json"]);
    assert_eq!(code, 0);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(parsed["type"], "commands.list");
}

#[test]
fn test_help_json_for_command() {
    let (code, stdout, _) = run(&["conv", "list", "--help", "--json"]);
    assert_eq!(code, 0);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(parsed["type"], "help");
    assert_eq!(parsed["command"], "conv list");
    assert!(parsed["usage"].as_str().unwrap().contains("conv list"));

    let (code, _, stderr) = run(&["not-a-command", "--help", "--json"]);
    assert_eq!(code, 1);
    assert!(stderr.contains("Help generation failed"));
}

#[test]
fn test_completions_shells() {
    for shell in ["bash", "zsh", "fish"] {
        let (code, stdout, _) = run(&["completions", shell]);
        assert_eq!(code, 0, "completions {} should succeed", shell);
        assert!(!stdout.is_empty());
    }
    let (code, _, _) = run(&["completions", "not-a-shell"]);
    assert_eq!(code, 1);
}

#[test]
fn test_removed_client_secret_flag_guidance() {
    let (code, _, stderr) = run(&[
        "config",
        "oauth",
        "set",
        "work",
        "--client-id",
        "1.2",
        "--redirect-uri",
        "http://127.0.0.1:8765/callback",
        "--scopes",
        "all",
        "--client-secret",
        "supersecret",
    ]);
    assert_eq!(code, 1);
    assert!(stderr.contains("--client-secret was removed for security"));
    assert!(stderr.contains("--client-secret-env"));
}

#[test]
fn test_conv_list_types_conflict_exits_one() {
    let (code, _, stderr) = run(&["conv", "list", "--types", "im", "--all"]);
    assert_eq!(code, 1);
    assert!(stderr.contains("--types cannot be used with"));
}

#[test]
fn test_msg_post_reply_broadcast_requires_thread_ts() {
    let (code, _, stderr) = run(&["msg", "post", "C123", "hi", "--reply-broadcast"]);
    assert_eq!(code, 1);
    assert!(stderr.contains("--reply-broadcast requires --thread-ts"));
}

#[test]
fn test_file_download_requires_id_or_url() {
    let (code, _, stderr) = run(&["file", "download"]);
    assert_eq!(code, 1);
    assert!(stderr.contains("Either <file_id> or --url must be provided"));
}

#[test]
fn test_tunnel_login_requires_interactive_mode() {
    let (code, _, stderr) = run(&["--non-interactive", "auth", "login", "--cloudflared"]);
    assert_eq!(code, 1);
    assert!(stderr.contains("requires interactive mode"));
}
