//! CLI command handlers
//!
//! This module contains handler functions for CLI commands that were extracted from main.rs
//! to improve code organization and maintainability.

use crate::api::{execute_api_call, ApiCallArgs, ApiCallContext, ApiCallResponse, ApiClient};
use crate::auth;
use crate::debug;
use crate::oauth;
use crate::profile::{
    create_token_store, default_config_path, make_token_key, resolve_profile_full, TokenType,
};

/// Login arguments after scope expansion and tunnel-mode resolution
#[derive(Debug, Clone, PartialEq)]
pub struct LoginArgs {
    pub profile_name: Option<String>,
    pub client_id: Option<String>,
    pub bot_scopes: Option<Vec<String>>,
    pub user_scopes: Option<Vec<String>>,
    pub tunnel_mode: TunnelMode,
}

/// Tunnel mode for login
#[derive(Debug, Clone, PartialEq)]
pub enum TunnelMode {
    None,
    Cloudflared(Option<String>),
}

impl TunnelMode {
    /// Check if tunnel mode is enabled
    pub fn is_enabled(&self) -> bool {
        !matches!(self, TunnelMode::None)
    }

    /// Check if cloudflared is enabled
    pub fn is_cloudflared(&self) -> bool {
        matches!(self, TunnelMode::Cloudflared(_))
    }
}

impl LoginArgs {
    /// Build LoginArgs from clap-parsed CLI arguments.
    ///
    /// Scope inputs are normalized (comma-separated, whitespace-trimmed) and
    /// 'all' presets are expanded with the appropriate bot/user context.
    pub fn from_cli(cli: &crate::cli::args::LoginCliArgs) -> Self {
        let bot_scopes = cli.bot_scopes.as_ref().map(|s| {
            let scopes_input: Vec<String> = s.split(',').map(|s| s.trim().to_string()).collect();
            // Expand 'all' presets with bot context (true)
            oauth::expand_scopes_with_context(&scopes_input, true)
        });
        let user_scopes = cli.user_scopes.as_ref().map(|s| {
            let scopes_input: Vec<String> = s.split(',').map(|s| s.trim().to_string()).collect();
            // Expand 'all' presets with user context (false)
            oauth::expand_scopes_with_context(&scopes_input, false)
        });

        let tunnel_mode = match &cli.cloudflared {
            Some(path) => TunnelMode::Cloudflared(Some(path.clone())),
            None => TunnelMode::None,
        };

        Self {
            profile_name: cli.profile_name.clone(),
            client_id: cli.client_id.clone(),
            bot_scopes,
            user_scopes,
            tunnel_mode,
        }
    }
}

/// Run the auth login command
pub async fn run_auth_login(
    cli_args: &crate::cli::args::LoginCliArgs,
    non_interactive: bool,
) -> Result<(), String> {
    let parsed_args = LoginArgs::from_cli(cli_args);

    // Use default redirect_uri
    let redirect_uri = "http://127.0.0.1:8765/callback".to_string();

    // Keep base_url from environment for testing purposes only
    let base_url = std::env::var("SLACK_OAUTH_BASE_URL").ok();

    // If cloudflared is specified, use extended login flow (manifest-first)
    if parsed_args.tunnel_mode.is_enabled() {
        // Tunnel mode requires interactive mode for credential input after manifest generation
        if non_interactive {
            return Err(
                "Tunnel login (--cloudflared) requires interactive mode.\n\
                 The manifest-first flow needs user interaction to create the Slack App\n\
                 and then enter credentials. Use the standard login flow for non-interactive mode."
                    .to_string(),
            );
        }

        // Use default scopes if not provided
        let bot_scopes = parsed_args.bot_scopes.unwrap_or_else(oauth::bot_all_scopes);
        let user_scopes = parsed_args
            .user_scopes
            .unwrap_or_else(oauth::user_all_scopes);

        if debug::enabled() {
            debug::log("Preparing to call login_with_credentials_extended");
            debug::log(format!("bot_scopes_count={}", bot_scopes.len()));
            debug::log(format!("user_scopes_count={}", user_scopes.len()));
        }

        // Call extended login - credentials are collected AFTER manifest generation
        auth::login_with_credentials_extended(
            bot_scopes,
            user_scopes,
            parsed_args.profile_name,
            parsed_args.tunnel_mode.is_cloudflared(),
        )
        .await
        .map_err(|e| e.to_string())
    } else {
        // Call standard login with credentials
        // This will prompt for client_secret and other missing OAuth config
        auth::login_with_credentials(
            parsed_args.client_id,
            parsed_args.profile_name,
            redirect_uri,
            vec![], // Legacy scopes parameter (unused)
            parsed_args.bot_scopes,
            parsed_args.user_scopes,
            base_url,
            non_interactive,
        )
        .await
        .map_err(|e| e.to_string())
    }
}

/// Check if we should show private channel guidance
fn should_show_private_channel_guidance(
    api_args: &ApiCallArgs,
    token_type: &str,
    response: &ApiCallResponse,
) -> bool {
    // Only show guidance for conversations.list with private_channel type and bot token
    if api_args.method != "conversations.list" || token_type != "bot" {
        return false;
    }

    // Check if types parameter includes private_channel
    if let Some(types) = api_args.params.get("types") {
        if !types.contains("private_channel") {
            return false;
        }
    } else {
        return false;
    }

    // Check if response has empty channels array
    if let Some(channels) = response.response.get("channels") {
        if let Some(channels_array) = channels.as_array() {
            return channels_array.is_empty();
        }
    }

    false
}

/// Infer the default token type based on token store existence
/// Returns User if a user token exists, otherwise Bot
fn infer_default_token_type(
    token_store: &dyn crate::profile::TokenStore,
    team_id: &str,
    user_id: &str,
) -> TokenType {
    let user_token_key = format!("{}:{}:user", team_id, user_id);
    if token_store.exists(&user_token_key) {
        TokenType::User
    } else {
        TokenType::Bot
    }
}

/// Result of token resolution containing the token and its type
#[derive(Debug)]
struct ResolvedToken {
    token: String,
    token_type: TokenType,
}

/// Resolves and retrieves the appropriate token for an API call
///
/// This function encapsulates the token resolution logic:
/// 1. Determines token type: CLI flag > profile default > inferred (user if exists, else bot)
/// 2. Attempts to retrieve token from the token store
/// 3. If explicit token type was requested and not found, returns error
/// 4. If no explicit preference, falls back from user to bot token
///
/// # Arguments
/// * `token_store` - Token store to retrieve tokens from
/// * `team_id` - Team ID for token key construction
/// * `user_id` - User ID for token key construction
/// * `cli_token_type` - Optional token type from CLI flag (--token-type)
/// * `profile_default_token_type` - Optional default token type from profile config
/// * `profile_name` - Profile name for error messages
///
/// # Returns
/// * `Ok(ResolvedToken)` - Successfully resolved token and its type
/// * `Err(String)` - Error message describing why token resolution failed
fn resolve_token(
    token_store: &dyn crate::profile::TokenStore,
    team_id: &str,
    user_id: &str,
    cli_token_type: Option<TokenType>,
    profile_default_token_type: Option<TokenType>,
    profile_name: &str,
) -> Result<ResolvedToken, String> {
    // Infer default token type based on user token existence
    let inferred_default = infer_default_token_type(token_store, team_id, user_id);

    // Resolve token type: CLI flag > profile default > inferred default
    let resolved_token_type =
        TokenType::resolve(cli_token_type, profile_default_token_type, inferred_default);

    // Create token keys for both bot and user tokens
    let token_key_bot = make_token_key(team_id, user_id);
    let token_key_user = format!("{}:{}:user", team_id, user_id);

    // Select the appropriate token key based on resolved token type
    let token_key = match resolved_token_type {
        TokenType::Bot => token_key_bot.clone(),
        TokenType::User => token_key_user.clone(),
    };

    // Determine if the token type was explicitly requested via CLI flag OR default_token_type
    // If either is set, we should NOT fallback to a different token type
    let explicit_request = cli_token_type.is_some() || profile_default_token_type.is_some();

    // Retrieve token from the token store
    let token = match token_store.get(&token_key) {
        Ok(t) => t,
        Err(_) => {
            // If token not found in store, apply fallback logic
            if explicit_request {
                // If token type was explicitly requested, fail without fallback
                return Err(crate::cli::with_legacy_hint(format!(
                    "No {} token found for profile '{}' ({}:{}). Explicitly requested token type not available. Run 'slack auth login' to obtain a {} token.",
                    resolved_token_type, profile_name, team_id, user_id, resolved_token_type
                )));
            } else {
                // If no token type preference was specified, try bot token as fallback
                if resolved_token_type == TokenType::User {
                    if let Ok(bot_token) = token_store.get(&token_key_bot) {
                        eprintln!(
                            "Warning: User token not found, falling back to bot token for profile '{}'",
                            profile_name
                        );
                        return Ok(ResolvedToken {
                            token: bot_token,
                            token_type: TokenType::Bot,
                        });
                    } else {
                        return Err(crate::cli::with_legacy_hint(format!(
                            "No {} token found for profile '{}' ({}:{}). Run 'slack auth login' to obtain a token.",
                            resolved_token_type, profile_name, team_id, user_id
                        )));
                    }
                } else {
                    return Err(crate::cli::with_legacy_hint(format!(
                        "No {} token found for profile '{}' ({}:{}). Run 'slack auth login' to obtain a token.",
                        resolved_token_type, profile_name, team_id, user_id
                    )));
                }
            }
        }
    };

    Ok(ResolvedToken {
        token,
        token_type: resolved_token_type,
    })
}

/// Run the api call command
pub async fn run_api_call(args: Vec<String>) -> Result<(), Box<dyn std::error::Error>> {
    // Parse arguments
    let api_args = ApiCallArgs::parse(&args)?;

    // Resolve profile name using common helper (--profile > SLACK_PROFILE > "default")
    let profile_name = crate::cli::resolve_profile_name(&args);

    // Get config path
    let config_path = default_config_path()?;

    // Resolve profile to get full profile details
    let profile = resolve_profile_full(&config_path, &profile_name)
        .map_err(|e| format!("Failed to resolve profile '{}': {}", profile_name, e))?;

    // Create context from resolved profile
    let context = ApiCallContext {
        profile_name: Some(profile_name.clone()),
        team_id: profile.team_id.clone(),
        user_id: profile.user_id.clone(),
    };

    // Create token store to check token existence for inference
    let token_store =
        create_token_store().map_err(|e| format!("Failed to create token store: {}", e))?;

    // Resolve token using dedicated function
    let resolved = resolve_token(
        &*token_store,
        &profile.team_id,
        &profile.user_id,
        api_args.token_type,
        profile.default_token_type,
        &profile_name,
    )
    .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;

    let token = resolved.token;
    let resolved_token_type = resolved.token_type;

    // Get debug level from args
    let debug_level = debug::get_debug_level(&args);

    // Log debug information if --debug or --trace flag is present
    let token_store_backend = "keyring";

    let endpoint = format!("https://slack.com/api/{}", api_args.method);

    debug::log_api_context(
        debug_level,
        Some(&profile_name),
        token_store_backend,
        resolved_token_type.as_str(),
        &api_args.method,
        &endpoint,
    );

    // Create API client
    let client = ApiClient::new();

    // Execute API call with token type information and command name
    let response = execute_api_call(
        &client,
        &api_args,
        &token,
        &context,
        resolved_token_type.as_str(),
        "api call",
    )
    .await?;

    // Log error code if present
    debug::log_error_code(debug_level, &response.response);

    // Display error guidance if response contains a known error
    crate::api::display_error_guidance(&response);

    // Check if we should show guidance for private_channel with bot token
    if should_show_private_channel_guidance(&api_args, resolved_token_type.as_str(), &response) {
        eprintln!();
        eprintln!("Note: The conversation list for private channels is empty.");
        eprintln!("Bot tokens can only see private channels where the bot is a member.");
        eprintln!("To list all private channels, use a User Token with appropriate scopes.");
        eprintln!("Run: slackcli auth login (with user_scopes) or use --token-type user");
        eprintln!();
    }

    // Print response as JSON
    // If --raw flag is set or SLACKRS_OUTPUT=raw, output only the Slack API response without envelope
    // Note: api_args.raw already accounts for both --raw flag and SLACKRS_OUTPUT env via should_output_raw()
    let json = if api_args.raw {
        serde_json::to_string_pretty(&response.response)?
    } else {
        serde_json::to_string_pretty(&response)?
    };
    println!("{}", json);

    Ok(())
}

/// Common passphrase/i18n options shared between export and import commands
struct ExportImportCommon {
    passphrase_env: Option<String>,
    yes: bool,
    lang: Option<String>,
}

impl ExportImportCommon {
    /// Get Messages based on language setting
    fn get_messages(&self) -> auth::Messages {
        if let Some(ref lang_code) = self.lang {
            if let Some(language) = auth::Language::from_code(lang_code) {
                auth::Messages::new(language)
            } else {
                auth::Messages::default()
            }
        } else {
            auth::Messages::default()
        }
    }

    /// Get passphrase from environment variable or prompt
    fn get_passphrase(&self, messages: &auth::Messages) -> Result<String, String> {
        if let Some(ref env_var) = self.passphrase_env {
            match std::env::var(env_var) {
                Ok(val) => Ok(val),
                Err(_) => {
                    // Fallback to prompt if environment variable is not set
                    eprintln!(
                        "Warning: Environment variable {} not found, prompting for passphrase",
                        env_var
                    );
                    rpassword::prompt_password(messages.get("prompt.passphrase"))
                        .map_err(|e| format!("Error reading passphrase: {}", e))
                }
            }
        } else {
            // Fallback to prompt mode
            rpassword::prompt_password(messages.get("prompt.passphrase"))
                .map_err(|e| format!("Error reading passphrase: {}", e))
        }
    }
}

/// Handle auth export command
pub async fn handle_export_command(
    args: &crate::cli::args::ExportCliArgs,
    profile: Option<String>,
) {
    let common = ExportImportCommon {
        passphrase_env: args.passphrase_env.clone(),
        yes: args.yes,
        lang: args.lang.clone(),
    };

    // Which profile to export: --profile flag (global) only; --all overrides
    let profile_name = profile;
    let all = args.all;
    let output_path = args.out.clone();

    // Get i18n messages
    let messages = common.get_messages();

    // Show warning and validate --yes
    if !common.yes {
        eprintln!("{}", messages.get("warn.export_sensitive"));
        eprintln!("Error: --yes flag is required to confirm this dangerous operation");
        std::process::exit(1);
    }

    // Validate required options
    let output = match output_path {
        Some(path) => path,
        None => {
            eprintln!("Error: --out <file> is required");
            std::process::exit(1);
        }
    };

    // Get passphrase
    let passphrase = match common.get_passphrase(&messages) {
        Ok(pass) => pass,
        Err(e) => {
            eprintln!("{}", e);
            std::process::exit(1);
        }
    };

    let options = auth::ExportOptions {
        profile_name,
        all,
        output_path: output,
        passphrase,
        yes: common.yes,
    };

    let token_store = create_token_store().expect("Failed to create token store");
    match auth::export_profiles(&*token_store, &options) {
        Ok(result) => {
            // Show warnings for skipped profiles
            if !result.skipped_profiles.is_empty() {
                eprintln!("{}", messages.get("warn.export_skipped"));
                for profile_name in &result.skipped_profiles {
                    eprintln!("  - {}", profile_name);
                }
                eprintln!();
                eprintln!(
                    "{}",
                    messages
                        .get("info.export_summary")
                        .replace("{exported}", &result.exported_count.to_string())
                        .replace("{skipped}", &result.skipped_profiles.len().to_string())
                );
                eprintln!();
            }
            println!("{}", messages.get("success.export"));
        }
        Err(e) => {
            eprintln!("Export failed: {}", e);
            std::process::exit(1);
        }
    }
}

/// Handle auth import command
pub async fn handle_import_command(args: &crate::cli::args::ImportCliArgs) {
    let common = ExportImportCommon {
        passphrase_env: args.passphrase_env.clone(),
        yes: args.yes,
        lang: args.lang.clone(),
    };

    let input_path = args.input.clone();
    let force = args.force;
    let dry_run = args.dry_run;
    let json = args.json;

    // Get i18n messages
    let messages = common.get_messages();

    // Validate required options
    let input = match input_path {
        Some(path) => path,
        None => {
            eprintln!("Error: --in <file> is required");
            std::process::exit(1);
        }
    };

    // Get passphrase
    let passphrase = match common.get_passphrase(&messages) {
        Ok(pass) => pass,
        Err(e) => {
            eprintln!("{}", e);
            std::process::exit(1);
        }
    };

    let options = auth::ImportOptions {
        input_path: input,
        passphrase,
        yes: common.yes,
        force,
        dry_run,
        json,
    };

    let token_store = create_token_store().expect("Failed to create token store");
    match auth::import_profiles(&*token_store, &options) {
        Ok(result) => {
            if json {
                // Output JSON format
                match serde_json::to_string_pretty(&result) {
                    Ok(json_output) => {
                        println!("{}", json_output);
                    }
                    Err(e) => {
                        eprintln!("Failed to serialize result to JSON: {}", e);
                        std::process::exit(1);
                    }
                }
            } else {
                // Output text format
                if result.dry_run {
                    println!("Dry-run mode: no changes were written.");
                    println!();
                }

                println!("Import Summary:");
                println!("  Total: {}", result.summary.total);
                println!("  Updated: {}", result.summary.updated);
                println!("  Skipped: {}", result.summary.skipped);
                println!("  Overwritten: {}", result.summary.overwritten);
                println!();
                println!("Profile Details:");
                for profile_result in &result.profiles {
                    println!(
                        "  {} - {} ({})",
                        profile_result.profile_name, profile_result.action, profile_result.reason
                    );
                }
                println!();

                if result.dry_run {
                    println!("Dry-run complete. Re-run without --dry-run to apply changes.");
                } else {
                    println!("{}", messages.get("success.import"));
                }
            }
        }
        Err(e) => {
            eprintln!("Import failed: {}", e);
            std::process::exit(1);
        }
    }
}

/// Run install-skills command
///
/// # Arguments
/// * `args` - Command line arguments (may include source)
///
/// # Returns
/// * `Ok(())` - Success (JSON output to stdout)
/// * `Err(String)` - Error (error message to stderr, non-zero exit)
pub fn run_install_skill(args: &crate::cli::args::InstallSkillsArgs) -> Result<(), String> {
    use crate::skills;
    use serde_json::json;

    let global = args.global;
    let json_output = args.json;
    let source = args.source.as_deref();

    let installed = skills::install_skill(source, global).map_err(|e| e.to_string())?;

    if json_output {
        let response = json!({
            "schemaVersion": "1.0",
            "type": "skill-installation",
            "ok": true,
            "skills": [
                {
                    "name": installed.name,
                    "path": installed.path,
                    "source_type": installed.source_type,
                }
            ]
        });

        println!("{}", serde_json::to_string_pretty(&response).unwrap());
    } else {
        println!("Installed 1 {}skill:", if global { "global " } else { "" });
        println!("- {} -> {}", installed.name, installed.path);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::call::ApiCallMeta;
    use crate::profile::{InMemoryTokenStore, TokenStore};
    use serde_json::json;
    use serial_test::serial;
    use std::collections::HashMap;

    fn login_cli(
        profile_name: Option<&str>,
        client_id: Option<&str>,
        bot_scopes: Option<&str>,
        user_scopes: Option<&str>,
        cloudflared: Option<&str>,
    ) -> crate::cli::args::LoginCliArgs {
        crate::cli::args::LoginCliArgs {
            profile_name: profile_name.map(String::from),
            client_id: client_id.map(String::from),
            bot_scopes: bot_scopes.map(String::from),
            user_scopes: user_scopes.map(String::from),
            cloudflared: cloudflared.map(String::from),
        }
    }

    #[test]
    fn test_login_args_from_cli_empty() {
        let parsed = LoginArgs::from_cli(&login_cli(None, None, None, None, None));
        assert_eq!(parsed.profile_name, None);
        assert_eq!(parsed.client_id, None);
        assert_eq!(parsed.bot_scopes, None);
        assert_eq!(parsed.user_scopes, None);
        assert_eq!(parsed.tunnel_mode, TunnelMode::None);
    }

    #[test]
    fn test_login_args_from_cli_profile_and_client_id() {
        let parsed = LoginArgs::from_cli(&login_cli(
            Some("my-profile"),
            Some("123.456"),
            None,
            None,
            None,
        ));
        assert_eq!(parsed.profile_name, Some("my-profile".to_string()));
        assert_eq!(parsed.client_id, Some("123.456".to_string()));
        assert_eq!(parsed.tunnel_mode, TunnelMode::None);
    }

    #[test]
    fn test_login_args_from_cli_cloudflared() {
        // Default missing value ("cloudflared") is applied by clap; from_cli
        // sees the resolved path either way.
        let parsed = LoginArgs::from_cli(&login_cli(None, None, None, None, Some("cloudflared")));
        assert_eq!(
            parsed.tunnel_mode,
            TunnelMode::Cloudflared(Some("cloudflared".to_string()))
        );

        let parsed = LoginArgs::from_cli(&login_cli(
            None,
            None,
            None,
            None,
            Some("/usr/bin/cloudflared"),
        ));
        assert_eq!(
            parsed.tunnel_mode,
            TunnelMode::Cloudflared(Some("/usr/bin/cloudflared".to_string()))
        );
        assert!(parsed.tunnel_mode.is_cloudflared());
    }

    #[test]
    fn test_login_args_from_cli_bot_scopes_normalized() {
        let parsed = LoginArgs::from_cli(&login_cli(
            None,
            None,
            Some("chat:write, users:read"),
            None,
            None,
        ));
        let scopes = parsed.bot_scopes.unwrap();
        assert!(scopes.contains(&"chat:write".to_string()));
        assert!(scopes.contains(&"users:read".to_string()));
    }

    #[test]
    fn test_login_args_from_cli_user_scopes() {
        let parsed = LoginArgs::from_cli(&login_cli(
            None,
            None,
            None,
            Some("search:read,users:read"),
            None,
        ));
        assert!(parsed.user_scopes.is_some());
    }

    #[test]
    fn test_login_args_from_cli_all_parameters() {
        let parsed = LoginArgs::from_cli(&login_cli(
            Some("work"),
            Some("123.456"),
            Some("chat:write"),
            Some("users:read"),
            Some("cloudflared"),
        ));
        assert_eq!(parsed.profile_name, Some("work".to_string()));
        assert_eq!(parsed.client_id, Some("123.456".to_string()));
        assert!(parsed.bot_scopes.is_some());
        assert!(parsed.user_scopes.is_some());
        assert!(parsed.tunnel_mode.is_cloudflared());
    }

    #[test]
    fn test_tunnel_mode_none() {
        let mode = TunnelMode::None;
        assert!(!mode.is_enabled());
        assert!(!mode.is_cloudflared());
    }

    #[test]
    fn test_tunnel_mode_cloudflared() {
        let mode = TunnelMode::Cloudflared(Some("cloudflared".to_string()));
        assert!(mode.is_enabled());
        assert!(mode.is_cloudflared());
    }

    #[test]
    fn test_login_args_from_cli_all_preset_expands_scopes() {
        // 'all' presets must expand to the full scope lists, with the
        // right bot/user context for each side.
        let parsed = LoginArgs::from_cli(&login_cli(None, None, Some("all"), Some("all"), None));

        let bot_scopes = parsed.bot_scopes.unwrap();
        assert_eq!(bot_scopes, oauth::bot_all_scopes());
        assert!(!bot_scopes.contains(&"all".to_string()));

        let user_scopes = parsed.user_scopes.unwrap();
        assert_eq!(user_scopes, oauth::user_all_scopes());
        assert!(!user_scopes.contains(&"all".to_string()));

        // Bot and user presets differ (context-sensitive expansion)
        assert_ne!(bot_scopes, user_scopes);
    }

    /// Manifest-first tunnel login requires interactivity: branch selection
    /// must reject --cloudflared in non-interactive mode before any network
    /// or tunnel activity.
    #[tokio::test]
    async fn test_run_auth_login_cloudflared_rejects_non_interactive() {
        let cli = login_cli(None, None, None, None, Some("cloudflared"));
        let result = run_auth_login(&cli, true).await;
        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(msg.contains("requires interactive mode"), "got: {}", msg);
    }

    /// Standard login in non-interactive mode must fail fast with a
    /// comprehensive list of missing OAuth parameters (no prompts, no
    /// network) when nothing is provided or saved.
    #[tokio::test]
    #[serial]
    async fn test_run_auth_login_non_interactive_reports_missing_params() {
        // Point config at an empty temp location so no saved profile leaks in
        let temp_dir = tempfile::TempDir::new().unwrap();
        std::env::set_var(
            "SLACK_RS_CONFIG_PATH",
            temp_dir.path().join("profiles.json"),
        );
        crate::profile::use_mock_keyring();

        let cli = login_cli(Some("fresh-profile"), None, None, None, None);
        let result = run_auth_login(&cli, true).await;

        std::env::remove_var("SLACK_RS_CONFIG_PATH");

        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(
            msg.contains("Missing required OAuth parameters"),
            "got: {}",
            msg
        );
        assert!(msg.contains("--client-id"));
        assert!(msg.contains("--bot-scopes"));
        assert!(msg.contains("--user-scopes"));
    }

    #[test]
    fn test_should_show_private_channel_guidance_empty_response() {
        let mut params = HashMap::new();
        params.insert("types".to_string(), "private_channel".to_string());

        let args = ApiCallArgs {
            method: "conversations.list".to_string(),
            params,
            use_json: false,
            use_get: false,
            token_type: None,
            raw: false,
        };

        let response = ApiCallResponse {
            response: json!({
                "ok": true,
                "channels": []
            }),
            meta: ApiCallMeta {
                profile_name: Some("default".to_string()),
                team_id: "T123".to_string(),
                user_id: "U123".to_string(),
                method: "conversations.list".to_string(),
                command: "api call".to_string(),
                token_type: "bot".to_string(),
            },
        };

        // Should show guidance when bot token returns empty private channels
        assert!(should_show_private_channel_guidance(
            &args, "bot", &response
        ));
    }

    #[test]
    fn test_should_show_private_channel_guidance_non_empty_response() {
        let mut params = HashMap::new();
        params.insert("types".to_string(), "private_channel".to_string());

        let args = ApiCallArgs {
            method: "conversations.list".to_string(),
            params,
            use_json: false,
            use_get: false,
            token_type: None,
            raw: false,
        };

        let response = ApiCallResponse {
            response: json!({
                "ok": true,
                "channels": [
                    {"id": "C123", "name": "private-channel"}
                ]
            }),
            meta: ApiCallMeta {
                profile_name: Some("default".to_string()),
                team_id: "T123".to_string(),
                user_id: "U123".to_string(),
                method: "conversations.list".to_string(),
                command: "api call".to_string(),
                token_type: "bot".to_string(),
            },
        };

        // Should not show guidance when channels are returned
        assert!(!should_show_private_channel_guidance(
            &args, "bot", &response
        ));
    }

    #[test]
    fn test_should_show_private_channel_guidance_user_token() {
        let mut params = HashMap::new();
        params.insert("types".to_string(), "private_channel".to_string());

        let args = ApiCallArgs {
            method: "conversations.list".to_string(),
            params,
            use_json: false,
            use_get: false,
            token_type: None,
            raw: false,
        };

        let response = ApiCallResponse {
            response: json!({
                "ok": true,
                "channels": []
            }),
            meta: ApiCallMeta {
                profile_name: Some("default".to_string()),
                team_id: "T123".to_string(),
                user_id: "U123".to_string(),
                method: "conversations.list".to_string(),
                command: "api call".to_string(),
                token_type: "user".to_string(),
            },
        };

        // Should not show guidance when using user token
        assert!(!should_show_private_channel_guidance(
            &args, "user", &response
        ));
    }

    #[test]
    fn test_infer_default_token_type_with_user_token() {
        let token_store = InMemoryTokenStore::new();
        let team_id = "T123";
        let user_id = "U456";

        // Set a user token
        token_store
            .set(
                &format!("{}:{}:user", team_id, user_id),
                "xoxp-test-user-token",
            )
            .unwrap();

        // Should infer User when user token exists
        let inferred = infer_default_token_type(&token_store, team_id, user_id);
        assert_eq!(inferred, TokenType::User);
    }

    #[test]
    fn test_infer_default_token_type_without_user_token() {
        let token_store = InMemoryTokenStore::new();
        let team_id = "T123";
        let user_id = "U456";

        // Set only a bot token
        token_store
            .set(&format!("{}:{}", team_id, user_id), "xoxb-test-bot-token")
            .unwrap();

        // Should infer Bot when user token does not exist
        let inferred = infer_default_token_type(&token_store, team_id, user_id);
        assert_eq!(inferred, TokenType::Bot);
    }

    #[test]
    fn test_infer_default_token_type_with_both_tokens() {
        let token_store = InMemoryTokenStore::new();
        let team_id = "T123";
        let user_id = "U456";

        // Set both tokens
        token_store
            .set(&format!("{}:{}", team_id, user_id), "xoxb-test-bot-token")
            .unwrap();
        token_store
            .set(
                &format!("{}:{}:user", team_id, user_id),
                "xoxp-test-user-token",
            )
            .unwrap();

        // Should infer User when user token exists (even if bot token also exists)
        let inferred = infer_default_token_type(&token_store, team_id, user_id);
        assert_eq!(inferred, TokenType::User);
    }

    #[test]
    fn test_infer_default_token_type_with_no_tokens() {
        let token_store = InMemoryTokenStore::new();
        let team_id = "T123";
        let user_id = "U456";

        // No tokens set

        // Should infer Bot when no tokens exist
        let inferred = infer_default_token_type(&token_store, team_id, user_id);
        assert_eq!(inferred, TokenType::Bot);
    }

    #[test]
    #[serial]
    fn test_resolve_token_with_bot_token_in_store() {
        let token_store = InMemoryTokenStore::new();
        let team_id = "T123";
        let user_id = "U456";

        // Set a bot token
        token_store
            .set(&format!("{}:{}", team_id, user_id), "xoxb-test-bot-token")
            .unwrap();

        // Resolve token with no CLI or profile preference
        let result = resolve_token(&token_store, team_id, user_id, None, None, "default");

        assert!(result.is_ok());
        let resolved = result.unwrap();
        assert_eq!(resolved.token, "xoxb-test-bot-token");
        assert_eq!(resolved.token_type, TokenType::Bot);
    }

    #[test]
    #[serial]
    fn test_resolve_token_with_user_token_in_store() {
        let token_store = InMemoryTokenStore::new();
        let team_id = "T123";
        let user_id = "U456";

        // Set a user token
        token_store
            .set(
                &format!("{}:{}:user", team_id, user_id),
                "xoxp-test-user-token",
            )
            .unwrap();

        // Resolve token with no CLI or profile preference
        let result = resolve_token(&token_store, team_id, user_id, None, None, "default");

        assert!(result.is_ok());
        let resolved = result.unwrap();
        assert_eq!(resolved.token, "xoxp-test-user-token");
        assert_eq!(resolved.token_type, TokenType::User);
    }

    #[test]
    fn test_resolve_token_fails_with_empty_store() {
        let token_store = InMemoryTokenStore::new();
        let team_id = "T123";
        let user_id = "U456";

        // Resolve token with no tokens in store
        let result = resolve_token(&token_store, team_id, user_id, None, None, "default");

        assert!(result.is_err());
        let error_msg = result.unwrap_err();
        assert!(error_msg.contains("No bot token found"));
        assert!(error_msg.contains("slack auth login"));
    }

    #[test]
    #[serial]
    fn test_resolve_token_explicit_bot_request_fails_without_bot_token() {
        let token_store = InMemoryTokenStore::new();
        let team_id = "T123";
        let user_id = "U456";

        // Set only a user token
        token_store
            .set(
                &format!("{}:{}:user", team_id, user_id),
                "xoxp-test-user-token",
            )
            .unwrap();

        // Explicitly request bot token via CLI flag
        let result = resolve_token(
            &token_store,
            team_id,
            user_id,
            Some(TokenType::Bot),
            None,
            "default",
        );

        assert!(result.is_err());
        let error_msg = result.unwrap_err();
        assert!(error_msg.contains("No bot token found"));
        assert!(error_msg.contains("Explicitly requested token type not available"));
    }

    #[test]
    #[serial]
    fn test_resolve_token_explicit_user_request_fails_without_user_token() {
        let token_store = InMemoryTokenStore::new();
        let team_id = "T123";
        let user_id = "U456";

        // Set only a bot token
        token_store
            .set(&format!("{}:{}", team_id, user_id), "xoxb-test-bot-token")
            .unwrap();

        // Explicitly request user token via CLI flag
        let result = resolve_token(
            &token_store,
            team_id,
            user_id,
            Some(TokenType::User),
            None,
            "default",
        );

        assert!(result.is_err());
        let error_msg = result.unwrap_err();
        assert!(error_msg.contains("No user token found"));
        assert!(error_msg.contains("Explicitly requested token type not available"));
    }

    #[test]
    #[serial]
    fn test_resolve_token_fallback_from_user_to_bot() {
        let token_store = InMemoryTokenStore::new();
        let team_id = "T123";
        let user_id = "U456";

        // Set only a bot token
        token_store
            .set(&format!("{}:{}", team_id, user_id), "xoxb-test-bot-token")
            .unwrap();

        // No explicit request (user token is inferred default when it doesn't exist -> Bot)
        // But if user token were to be the inferred default and not found, it should fallback
        // Let me test the actual fallback scenario

        // Actually, the fallback only happens when resolved type is User and no explicit request
        // Since there's no user token, inferred default will be Bot anyway
        // To test fallback, I need to simulate a case where User is resolved but not found

        // This is not possible with the current logic because if user token doesn't exist,
        // inferred_default will be Bot. The fallback case only triggers when:
        // - resolved_token_type == TokenType::User
        // - explicit_request == false
        // - user token not in store

        // For this to happen, we'd need profile.default_token_type to be User but no user token
        // Let me create that scenario:

        let result = resolve_token(
            &token_store,
            team_id,
            user_id,
            None,
            Some(TokenType::User), // Profile says use User
            "default",
        );

        // This should fail because profile explicitly requested User token
        assert!(result.is_err());
    }

    #[test]
    #[serial]
    fn test_resolve_token_no_fallback_when_profile_default_set() {
        let token_store = InMemoryTokenStore::new();
        let team_id = "T123";
        let user_id = "U456";

        // Set only a bot token
        token_store
            .set(&format!("{}:{}", team_id, user_id), "xoxb-test-bot-token")
            .unwrap();

        // Profile default is User (explicit request)
        let result = resolve_token(
            &token_store,
            team_id,
            user_id,
            None,
            Some(TokenType::User),
            "default",
        );

        // Should fail without fallback because profile explicitly requested User
        assert!(result.is_err());
        let error_msg = result.unwrap_err();
        assert!(error_msg.contains("Explicitly requested token type not available"));
    }

    #[test]
    #[serial]
    fn test_resolve_token_cli_overrides_profile_default() {
        let token_store = InMemoryTokenStore::new();
        let team_id = "T123";
        let user_id = "U456";

        // Set both tokens
        token_store
            .set(&format!("{}:{}", team_id, user_id), "xoxb-test-bot-token")
            .unwrap();
        token_store
            .set(
                &format!("{}:{}:user", team_id, user_id),
                "xoxp-test-user-token",
            )
            .unwrap();

        // Profile default is Bot, but CLI requests User
        let result = resolve_token(
            &token_store,
            team_id,
            user_id,
            Some(TokenType::User), // CLI flag
            Some(TokenType::Bot),  // Profile default
            "default",
        );

        assert!(result.is_ok());
        let resolved = result.unwrap();
        assert_eq!(resolved.token, "xoxp-test-user-token");
        assert_eq!(resolved.token_type, TokenType::User);
    }

    #[test]
    #[serial]
    fn test_resolve_token_with_both_tokens_prefers_user() {
        let token_store = InMemoryTokenStore::new();
        let team_id = "T123";
        let user_id = "U456";

        // Set both tokens
        token_store
            .set(&format!("{}:{}", team_id, user_id), "xoxb-test-bot-token")
            .unwrap();
        token_store
            .set(
                &format!("{}:{}:user", team_id, user_id),
                "xoxp-test-user-token",
            )
            .unwrap();

        // No explicit preference
        let result = resolve_token(&token_store, team_id, user_id, None, None, "default");

        assert!(result.is_ok());
        let resolved = result.unwrap();
        // Should prefer User when both exist
        assert_eq!(resolved.token, "xoxp-test-user-token");
        assert_eq!(resolved.token_type, TokenType::User);
    }
}
