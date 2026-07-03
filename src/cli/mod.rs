//! CLI command routing and handlers

pub mod args;
mod context;
mod handlers;
pub mod introspect;

pub use args::Cli;
pub use context::CliContext;
pub use handlers::{
    handle_export_command, handle_import_command, run_api_call, run_auth_login, run_install_skill,
};
pub use introspect::{
    generate_commands_list, generate_help, generate_schema, CommandDef, CommandsListResponse,
    HelpResponse, SchemaResponse,
};

use crate::api::{ApiClient, CommandResponse};
use crate::commands;
use crate::commands::ConversationSelector;
use crate::debug;
use crate::profile::{
    create_token_store, default_config_path, load_config, make_token_key, resolve_profile_full,
    TokenStore, TokenType,
};
use serde_json::Value;

/// Resolved global CLI options shared by all commands.
#[derive(Debug, Clone)]
pub struct GlobalArgs {
    /// Raw `--profile` flag value (before env/default fallback)
    pub profile: Option<String>,
    /// Non-interactive mode (explicit flag or auto-detected non-TTY stdin)
    pub non_interactive: bool,
    /// Debug level resolved from --debug/--trace/SLACK_RS_DEBUG
    pub debug_level: debug::DebugLevel,
}

impl GlobalArgs {
    /// Resolve profile name: `--profile` flag > `SLACK_PROFILE` env > "default"
    pub fn profile_name(&self) -> String {
        self.profile
            .clone()
            .or_else(|| std::env::var("SLACK_PROFILE").ok())
            .unwrap_or_else(|| "default".to_string())
    }
}

/// Resolve token from the token store
///
/// # Arguments
/// * `token_store` - Token store to retrieve tokens from
/// * `token_key` - Key to use for token store lookup
/// * `fallback_token_key` - Optional fallback key (e.g., bot token when user token not found)
/// * `explicit_request` - Whether the token type was explicitly requested (via --token-type or default_token_type)
///
/// # Returns
/// * `Ok(token)` - Successfully resolved token
/// * `Err(message)` - Token resolution failed
///
/// # Token Resolution Priority
/// 1. Token store with primary token_key
/// 2. Token store with fallback_token_key (only if not explicit_request)
/// 3. Error if no token found
#[allow(dead_code)]
pub fn resolve_token_for_wrapper(
    token_store: &dyn TokenStore,
    token_key: &str,
    fallback_token_key: Option<&str>,
    explicit_request: bool,
) -> Result<String, String> {
    // Priority 1: Token store with primary key
    if let Ok(token) = token_store.get(token_key) {
        return Ok(token);
    }

    // Priority 2: Fallback token (only if not explicit_request)
    if !explicit_request {
        if let Some(fallback_key) = fallback_token_key {
            if let Ok(token) = token_store.get(fallback_key) {
                eprintln!("Warning: Primary token not found, falling back to alternative token");
                return Ok(token);
            }
        }
    }

    // Priority 3: Error
    let base = if explicit_request {
        "No token found for explicitly requested token type. Run 'slack auth login' to obtain a token."
    } else {
        "No token found. Run 'slack auth login' to obtain a token."
    };
    Err(with_legacy_hint(base.to_string()))
}

/// Append a legacy-tokens.json migration hint to a token error message
/// when an unmigrated plaintext token file still exists.
pub(crate) fn with_legacy_hint(message: String) -> String {
    match crate::profile::legacy_tokens_hint() {
        Some(hint) => format!("{}\n{}", message, hint),
        None => message,
    }
}

/// Get API client for a profile with optional token type selection
///
/// # Arguments
/// * `profile_name` - Optional profile name (defaults to "default")
/// * `token_type` - Optional token type (bot/user). If None, uses profile default or bot fallback
///
/// # Token Resolution Priority
/// 1. CLI flag token_type parameter (if provided)
/// 2. Profile's default_token_type (if set)
/// 3. Try user token first, fall back to bot token
pub async fn get_api_client_with_token_type(
    profile_name: Option<String>,
    token_type: Option<TokenType>,
) -> Result<ApiClient, String> {
    let profile_name = profile_name.unwrap_or_else(|| "default".to_string());
    let config_path = default_config_path().map_err(|e| e.to_string())?;
    let config = load_config(&config_path).map_err(|e| e.to_string())?;

    let profile = config
        .get(&profile_name)
        .ok_or_else(|| format!("Profile '{}' not found", profile_name))?;

    let token_store = create_token_store().map_err(|e| e.to_string())?;

    // Resolve token type: CLI flag > profile default > try user first with bot fallback
    let resolved_token_type = token_type.or(profile.default_token_type);

    let bot_token_key = make_token_key(&profile.team_id, &profile.user_id);
    let user_token_key = format!("{}:{}:user", profile.team_id, profile.user_id);

    let token = match resolved_token_type {
        Some(TokenType::Bot) => {
            // Explicitly requested bot token
            token_store
                .get(&bot_token_key)
                .map_err(|e| with_legacy_hint(format!("Failed to get bot token: {}", e)))?
        }
        Some(TokenType::User) => {
            // Explicitly requested user token
            token_store
                .get(&user_token_key)
                .map_err(|e| with_legacy_hint(format!("Failed to get user token: {}", e)))?
        }
        None => {
            // No explicit preference, try user token first (for APIs that require user scope)
            match token_store.get(&user_token_key) {
                Ok(user_token) => user_token,
                Err(_) => {
                    // Fall back to bot token
                    token_store
                        .get(&bot_token_key)
                        .map_err(|e| with_legacy_hint(format!("Failed to get token: {}", e)))?
                }
            }
        }
    };

    Ok(ApiClient::with_token(token))
}

/// Get API client for a profile (legacy function, maintains backward compatibility)
#[allow(dead_code)]
pub async fn get_api_client(profile_name: Option<String>) -> Result<ApiClient, String> {
    get_api_client_with_token_type(profile_name, None).await
}

/// Check if a flag exists in args
pub fn has_flag(args: &[String], flag: &str) -> bool {
    args.iter().any(|arg| arg == flag)
}

/// Determine if output should be raw based on the --raw flag and the
/// SLACKRS_OUTPUT environment variable.
///
/// # Priority
/// 1. --raw flag (highest priority)
/// 2. SLACKRS_OUTPUT environment variable ("raw" or "envelope")
/// 3. Default to envelope (false)
pub fn should_output_raw(raw_flag: bool) -> bool {
    // Priority 1: --raw flag always wins
    if raw_flag {
        return true;
    }

    // Priority 2: Check SLACKRS_OUTPUT environment variable
    if let Ok(output_mode) = std::env::var("SLACKRS_OUTPUT") {
        return output_mode.trim().to_lowercase() == "raw";
    }

    // Priority 3: Default to envelope (false)
    false
}

/// Check if error message indicates non-interactive mode failure
pub fn is_non_interactive_error(error_msg: &str) -> bool {
    error_msg.contains("Non-interactive mode error")
        || error_msg.contains("Use --yes flag to confirm in non-interactive mode")
}

/// Wrap response with unified envelope including metadata
#[allow(dead_code)]
pub async fn wrap_with_envelope(
    response: Value,
    method: &str,
    command: &str,
    profile_name: Option<String>,
) -> Result<CommandResponse, String> {
    wrap_with_envelope_and_token_type(response, method, command, profile_name, None).await
}

/// Wrap response with unified envelope including metadata and explicit token type
pub async fn wrap_with_envelope_and_token_type(
    response: Value,
    method: &str,
    command: &str,
    profile_name: Option<String>,
    explicit_token_type: Option<TokenType>,
) -> Result<CommandResponse, String> {
    let profile_name_str = profile_name.unwrap_or_else(|| "default".to_string());
    let config_path = default_config_path().map_err(|e| e.to_string())?;
    let profile = resolve_profile_full(&config_path, &profile_name_str)
        .map_err(|e| format!("Failed to resolve profile '{}': {}", profile_name_str, e))?;

    // Resolve token type for metadata
    let token_type_str = if let Some(explicit) = explicit_token_type {
        // If explicitly specified via --token-type, use that
        Some(explicit.to_string())
    } else {
        // Resolve from token store (check which token exists)
        let token_store = create_token_store().map_err(|e| e.to_string())?;
        let bot_token_key = make_token_key(&profile.team_id, &profile.user_id);
        let user_token_key = format!("{}:{}:user", profile.team_id, profile.user_id);

        // Try to determine which token was used based on default_token_type
        let resolved_type = profile.default_token_type.or_else(|| {
            // If no default, check which token exists (try user first, then bot)
            if token_store.get(&user_token_key).is_ok() {
                Some(TokenType::User)
            } else if token_store.get(&bot_token_key).is_ok() {
                Some(TokenType::Bot)
            } else {
                None
            }
        });

        resolved_type.map(|t| t.to_string())
    };

    Ok(CommandResponse::with_token_type(
        response,
        Some(profile_name_str),
        profile.team_id,
        profile.user_id,
        method.to_string(),
        command.to_string(),
        token_type_str,
    ))
}

/// Resolve profile name with priority: --profile flag > SLACK_PROFILE env > "default"
///
/// This helper works on a raw argv slice and is used by the free-form
/// `api call` argument list, where `--profile` may appear among trailing
/// key=value parameters. Typed commands use [`GlobalArgs::profile_name`].
pub fn resolve_profile_name(args: &[String]) -> String {
    // Priority 1: Check for --profile flag in args
    if let Some(profile) = get_option(args, "--profile=") {
        return profile;
    }

    // Priority 2: Check SLACK_PROFILE environment variable
    if let Ok(profile) = std::env::var("SLACK_PROFILE") {
        return profile;
    }

    // Priority 3: Default to "default"
    "default".to_string()
}

/// Load the workspace users cache for a profile, returning None gracefully on any error.
///
/// This is used by history/thread enrichment to resolve user names. If the cache file
/// doesn't exist, the profile isn't found, or any other error occurs, `None` is returned
/// and enrichment still populates user IDs (without names).
fn resolve_workspace_cache(profile_name: &str) -> Option<commands::users_cache::WorkspaceCache> {
    let config_path = default_config_path().ok()?;
    let config = load_config(&config_path).ok()?;
    let profile = config.get(profile_name)?;
    let cache_path = commands::users_cache::UsersCacheFile::default_path().ok()?;
    let cache_file = commands::users_cache::UsersCacheFile::load(&cache_path).ok()?;
    cache_file.caches.get(&profile.team_id).cloned()
}

/// Get option value from args
/// Supports both --key=value and --key value formats
/// When using space-separated format, value must not start with '-'
pub fn get_option(args: &[String], prefix: &str) -> Option<String> {
    // First try --key=value format
    if let Some(value) = args
        .iter()
        .find(|arg| arg.starts_with(prefix))
        .and_then(|arg| arg.strip_prefix(prefix))
        .map(|s| s.to_string())
    {
        return Some(value);
    }

    // Then try --key value format (space-separated)
    // Extract the flag name without the '=' suffix
    let flag = prefix.strip_suffix('=').unwrap_or(prefix);
    if let Some(pos) = args.iter().position(|arg| arg == flag) {
        if let Some(value) = args.get(pos + 1) {
            // Only treat as value if it doesn't start with '-'
            if !value.starts_with('-') {
                return Some(value.clone());
            }
        }
    }

    None
}

/// Get all options with a specific prefix from args
/// Supports both --key=value and --key value formats (can be mixed)
/// When using space-separated format, value must not start with '-'
pub fn get_all_options(args: &[String], prefix: &str) -> Vec<String> {
    let mut results = Vec::new();

    // Collect --key=value format
    results.extend(
        args.iter()
            .filter(|arg| arg.starts_with(prefix))
            .filter_map(|arg| arg.strip_prefix(prefix))
            .map(|s| s.to_string()),
    );

    // Collect --key value format (space-separated)
    let flag = prefix.strip_suffix('=').unwrap_or(prefix);
    let mut i = 0;
    while i < args.len() {
        if args[i] == flag {
            if let Some(value) = args.get(i + 1) {
                // Only treat as value if it doesn't start with '-'
                if !value.starts_with('-') {
                    results.push(value.clone());
                    i += 2; // Skip both flag and value
                    continue;
                }
            }
        }
        i += 1;
    }

    results
}

/// Resolve the effective token type for debug logging.
///
/// Priority: explicit CLI flag > profile default > inferred from token
/// availability (user token if present, otherwise bot).
fn resolve_token_type_for_debug(
    profile_name: &str,
    token_type: Option<TokenType>,
) -> Result<TokenType, String> {
    if let Some(explicit) = token_type {
        return Ok(explicit);
    }

    let config_path = default_config_path().map_err(|e| e.to_string())?;
    let profile = resolve_profile_full(&config_path, profile_name)
        .map_err(|e| format!("Failed to resolve profile '{}': {}", profile_name, e))?;

    if let Some(default_type) = profile.default_token_type {
        return Ok(default_type);
    }

    let token_store = create_token_store().map_err(|e| e.to_string())?;
    let user_token_key = format!("{}:{}:user", profile.team_id, profile.user_id);
    if token_store.get(&user_token_key).is_ok() {
        Ok(TokenType::User)
    } else {
        Ok(TokenType::Bot)
    }
}

pub async fn run_search(cmd: &args::SearchArgs, globals: &GlobalArgs) -> Result<(), String> {
    let query = cmd.query.clone();
    let profile_name = globals.profile_name();
    let token_type = cmd.token_type;
    let raw = should_output_raw(cmd.raw);

    let client = get_api_client_with_token_type(Some(profile_name.clone()), token_type).await?;
    let response = commands::search(
        &client,
        query,
        cmd.count,
        cmd.page,
        cmd.sort.clone(),
        cmd.sort_dir.clone(),
    )
    .await
    .map_err(|e| e.to_string())?;

    // Display error guidance if response contains a known error
    crate::api::display_wrapper_error_guidance(&response);

    // Output with or without envelope
    let output = if raw {
        serde_json::to_string_pretty(&response).unwrap()
    } else {
        let response_value = serde_json::to_value(&response).map_err(|e| e.to_string())?;
        let wrapped = wrap_with_envelope_and_token_type(
            response_value,
            "search.messages",
            "search",
            Some(profile_name),
            token_type,
        )
        .await?;
        serde_json::to_string_pretty(&wrapped).unwrap()
    };

    println!("{}", output);
    Ok(())
}

pub async fn run_conv_list(cmd: &args::ConvListArgs, globals: &GlobalArgs) -> Result<(), String> {
    let profile_name = globals.profile_name();
    let token_type = cmd.token_type;
    let raw = should_output_raw(cmd.raw);

    // Validate: --types is mutually exclusive with --include-private and --all
    if cmd.types.is_some() && (cmd.include_private || cmd.all) {
        return Err("Error: --types cannot be used with --include-private or --all".to_string());
    }

    // Resolve types based on flags
    let resolved_types = if let Some(explicit_types) = cmd.types.clone() {
        // User explicitly specified types
        Some(explicit_types)
    } else if cmd.all {
        // --all flag: include all conversation types
        Some("public_channel,private_channel,im,mpim".to_string())
    } else {
        // Default (and --include-private): public and private channels
        Some("public_channel,private_channel".to_string())
    };

    // Parse format option (default: json)
    let format = if let Some(fmt_str) = &cmd.format {
        commands::OutputFormat::parse(fmt_str)?
    } else {
        commands::OutputFormat::Json
    };

    // Validate --raw compatibility
    if raw && format != commands::OutputFormat::Json {
        return Err(format!(
            "--raw is only valid with --format json, but got --format {}",
            format
        ));
    }

    // Parse sort options
    let sort_key = if let Some(sort_str) = &cmd.sort {
        Some(commands::SortKey::parse(sort_str)?)
    } else {
        None
    };

    let sort_dir = if let Some(dir_str) = &cmd.sort_dir {
        commands::SortDirection::parse(dir_str)?
    } else {
        commands::SortDirection::default()
    };

    // Parse filters
    let filters: Result<Vec<_>, _> = cmd
        .filter
        .iter()
        .map(|s| commands::ConversationFilter::parse(s))
        .collect();
    let filters = filters.map_err(|e| e.to_string())?;

    // Log debug information if --debug or --trace flag is present
    let debug_level = globals.debug_level;
    let token_store_backend = "keyring";
    let resolved_token_type = resolve_token_type_for_debug(&profile_name, token_type)?;
    let endpoint = "https://slack.com/api/conversations.list";

    debug::log_api_context(
        debug_level,
        Some(&profile_name),
        token_store_backend,
        resolved_token_type.as_str(),
        "conversations.list",
        endpoint,
    );

    let client = get_api_client_with_token_type(Some(profile_name.clone()), token_type).await?;
    let mut response = commands::conv_list(&client, resolved_types, cmd.limit)
        .await
        .map_err(|e| e.to_string())?;

    // Log error code if present
    debug::log_error_code(
        debug_level,
        &serde_json::to_value(&response).unwrap_or_default(),
    );

    // Display error guidance if response contains a known error
    crate::api::display_wrapper_error_guidance(&response);

    // Apply filters
    commands::apply_filters(&mut response, &filters);

    // Apply sorting if specified
    if let Some(key) = sort_key {
        commands::sort_conversations(&mut response, key, sort_dir);
    }

    // Format output: non-JSON formats bypass raw/envelope logic
    let output = if format != commands::OutputFormat::Json {
        commands::format_response(&response, format)?
    } else if raw {
        serde_json::to_string_pretty(&response).unwrap()
    } else {
        let response_value = serde_json::to_value(&response).map_err(|e| e.to_string())?;
        let wrapped = wrap_with_envelope_and_token_type(
            response_value,
            "conversations.list",
            "conv list",
            Some(profile_name),
            token_type,
        )
        .await?;
        serde_json::to_string_pretty(&wrapped).unwrap()
    };

    println!("{}", output);
    Ok(())
}

pub async fn run_conv_select(
    cmd: &args::ConvSelectArgs,
    globals: &GlobalArgs,
) -> Result<(), String> {
    let profile_name = globals.profile_name();
    let token_type = cmd.token_type;

    // Parse filters
    let filters: Result<Vec<_>, _> = cmd
        .filter
        .iter()
        .map(|s| commands::ConversationFilter::parse(s))
        .collect();
    let filters = filters.map_err(|e| e.to_string())?;

    // Resolve types: default to public_channel,private_channel if not specified
    let resolved_types = cmd
        .types
        .clone()
        .or(Some("public_channel,private_channel".to_string()));

    let client = get_api_client_with_token_type(Some(profile_name), token_type).await?;
    let mut response = commands::conv_list(&client, resolved_types, cmd.limit)
        .await
        .map_err(|e| e.to_string())?;

    // Apply filters
    commands::apply_filters(&mut response, &filters);

    // Extract conversations and present selection
    let items = commands::extract_conversations(&response);
    let selector = commands::StdinSelector;
    let channel_id = selector.select(&items)?;

    println!("{}", channel_id);
    Ok(())
}

pub async fn run_conv_search(
    cmd: &args::ConvSearchArgs,
    globals: &GlobalArgs,
) -> Result<(), String> {
    let pattern = cmd.pattern.clone();
    let profile_name = globals.profile_name();
    let token_type = cmd.token_type;
    let raw = should_output_raw(cmd.raw);

    // Parse format option (default: json)
    let format = if let Some(fmt_str) = &cmd.format {
        commands::OutputFormat::parse(fmt_str)?
    } else {
        commands::OutputFormat::Json
    };

    // Validate --raw compatibility
    if raw && format != commands::OutputFormat::Json {
        return Err(format!(
            "--raw is only valid with --format json, but got --format {}",
            format
        ));
    }

    // Parse sort options
    let sort_key = if let Some(sort_str) = &cmd.sort {
        Some(commands::SortKey::parse(sort_str)?)
    } else {
        None
    };

    let sort_dir = if let Some(dir_str) = &cmd.sort_dir {
        commands::SortDirection::parse(dir_str)?
    } else {
        commands::SortDirection::default()
    };

    // Build filters: inject name:<pattern> filter + any additional filters
    let mut filters: Vec<commands::ConversationFilter> =
        vec![commands::ConversationFilter::Name(pattern)];

    // Parse and add additional filters
    for filter_str in &cmd.filter {
        filters.push(commands::ConversationFilter::parse(filter_str).map_err(|e| e.to_string())?);
    }

    // Resolve types: default to public_channel,private_channel if not specified
    let resolved_types = cmd
        .types
        .clone()
        .or(Some("public_channel,private_channel".to_string()));

    let client = get_api_client_with_token_type(Some(profile_name.clone()), token_type).await?;
    let mut response = commands::conv_list(&client, resolved_types, cmd.limit)
        .await
        .map_err(|e| e.to_string())?;

    // Apply filters
    commands::apply_filters(&mut response, &filters);

    // Apply sorting if specified
    if let Some(key) = sort_key {
        commands::sort_conversations(&mut response, key, sort_dir);
    }

    // If --select flag is present, use interactive selection
    if cmd.select {
        let items = commands::extract_conversations(&response);
        let selector = commands::StdinSelector;
        let channel_id = selector.select(&items)?;
        println!("{}", channel_id);
        return Ok(());
    }

    // Format output: non-JSON formats bypass raw/envelope logic
    let output = if format != commands::OutputFormat::Json {
        commands::format_response(&response, format)?
    } else if raw {
        serde_json::to_string_pretty(&response).unwrap()
    } else {
        let response_value = serde_json::to_value(&response).map_err(|e| e.to_string())?;
        let wrapped = wrap_with_envelope_and_token_type(
            response_value,
            "conversations.list",
            "conv search",
            Some(profile_name),
            token_type,
        )
        .await?;
        serde_json::to_string_pretty(&wrapped).unwrap()
    };

    println!("{}", output);
    Ok(())
}

pub async fn run_conv_history(
    cmd: &args::ConvHistoryArgs,
    globals: &GlobalArgs,
) -> Result<(), String> {
    let profile_name = globals.profile_name();
    let token_type = cmd.token_type;

    let channel = if cmd.interactive {
        // Use conv_select logic to get channel
        // Parse filters
        let filters: Result<Vec<_>, _> = cmd
            .filter
            .iter()
            .map(|s| commands::ConversationFilter::parse(s))
            .collect();
        let filters = filters.map_err(|e| e.to_string())?;

        // Resolve types: default to public_channel,private_channel if not specified
        let resolved_types = cmd
            .types
            .clone()
            .or(Some("public_channel,private_channel".to_string()));

        let client = get_api_client_with_token_type(Some(profile_name.clone()), token_type).await?;
        let mut response = commands::conv_list(&client, resolved_types, None)
            .await
            .map_err(|e| e.to_string())?;

        // Apply filters
        commands::apply_filters(&mut response, &filters);

        // Extract conversations and present selection
        let items = commands::extract_conversations(&response);
        let selector = commands::StdinSelector;
        selector.select(&items)?
    } else {
        cmd.channel
            .clone()
            .ok_or_else(|| "Channel argument required when --interactive is not used".to_string())?
    };

    let raw = should_output_raw(cmd.raw);

    // Log debug information if --debug or --trace flag is present
    let debug_level = globals.debug_level;
    let token_store_backend = "keyring";
    let resolved_token_type = resolve_token_type_for_debug(&profile_name, token_type)?;
    let endpoint = "https://slack.com/api/conversations.history";

    debug::log_api_context(
        debug_level,
        Some(&profile_name),
        token_store_backend,
        resolved_token_type.as_str(),
        "conversations.history",
        endpoint,
    );

    let client = get_api_client_with_token_type(Some(profile_name.clone()), token_type).await?;
    let mut response = commands::conv_history(
        &client,
        channel,
        cmd.limit,
        cmd.oldest.clone(),
        cmd.latest.clone(),
    )
    .await
    .map_err(|e| e.to_string())?;

    // Log error code if present
    debug::log_error_code(
        debug_level,
        &serde_json::to_value(&response).unwrap_or_default(),
    );

    // Display error guidance if response contains a known error
    crate::api::display_wrapper_error_guidance(&response);

    // Enrich messages with user info from workspace cache (graceful: no cache → ids only)
    let workspace_cache = resolve_workspace_cache(&profile_name);
    if let Some(messages) = response.data.get_mut("messages") {
        if let Some(msgs) = messages.as_array_mut() {
            for msg in msgs {
                commands::conv::enrich_message_with_users(msg, workspace_cache.as_ref());
            }
        }
    }

    // Output with or without envelope
    let output = if raw {
        serde_json::to_string_pretty(&response).unwrap()
    } else {
        let response_value = serde_json::to_value(&response).map_err(|e| e.to_string())?;
        let wrapped = wrap_with_envelope_and_token_type(
            response_value,
            "conversations.history",
            "conv history",
            Some(profile_name),
            token_type,
        )
        .await?;
        serde_json::to_string_pretty(&wrapped).unwrap()
    };

    println!("{}", output);
    Ok(())
}

/// Add default `thread get` response-level user metadata without mutating Slack messages.
pub async fn add_thread_resolution_metadata(
    client: &ApiClient,
    response: &mut crate::api::ApiResponse,
    messages: &[Value],
    workspace_cache: Option<&commands::users_cache::WorkspaceCache>,
) -> Result<(), crate::api::ApiError> {
    let (resolved_users, unresolved_user_ids) =
        commands::conv::resolve_thread_users(client, messages, workspace_cache).await?;

    response.data.insert(
        "resolved_users".to_string(),
        serde_json::to_value(resolved_users).map_err(crate::api::ApiError::JsonError)?,
    );
    if !unresolved_user_ids.is_empty() {
        response.data.insert(
            "unresolved_user_ids".to_string(),
            serde_json::to_value(unresolved_user_ids).map_err(crate::api::ApiError::JsonError)?,
        );
    }

    Ok(())
}

/// Build `thread get` JSON output using the same raw/default branching as the CLI command.
///
/// Tests inject a mock-backed [`ApiClient`] here so the command-output path can be verified
/// without relying on global token stores or process stdout capture.
#[allow(clippy::too_many_arguments)]
pub async fn build_thread_get_output(
    client: &ApiClient,
    channel: String,
    thread_ts: String,
    limit: Option<u32>,
    inclusive: Option<bool>,
    raw: bool,
    profile_name: String,
    token_type: Option<TokenType>,
) -> Result<Value, String> {
    let mut response = commands::thread_get(client, channel, thread_ts, limit, inclusive)
        .await
        .map_err(|e| e.to_string())?;

    crate::api::display_wrapper_error_guidance(&response);

    if raw {
        return serde_json::to_value(&response).map_err(|e| e.to_string());
    }

    let messages = response
        .data
        .get("messages")
        .and_then(|messages| messages.as_array())
        .cloned()
        .unwrap_or_default();
    let workspace_cache = resolve_workspace_cache(&profile_name);

    add_thread_resolution_metadata(client, &mut response, &messages, workspace_cache.as_ref())
        .await
        .map_err(|e| e.to_string())?;

    let response_value = serde_json::to_value(&response).map_err(|e| e.to_string())?;
    let wrapped = wrap_with_envelope_and_token_type(
        response_value,
        "conversations.replies",
        "thread get",
        Some(profile_name),
        token_type,
    )
    .await?;
    serde_json::to_value(wrapped).map_err(|e| e.to_string())
}

pub async fn run_thread_get(cmd: &args::ThreadGetArgs, globals: &GlobalArgs) -> Result<(), String> {
    let channel = cmd.channel.clone();
    let thread_ts = cmd.thread_ts.clone();
    let inclusive = if cmd.inclusive { Some(true) } else { None };
    let profile_name = globals.profile_name();
    let token_type = cmd.token_type;
    let raw = should_output_raw(cmd.raw);

    // Log debug information if --debug or --trace flag is present
    let debug_level = globals.debug_level;
    let token_store_backend = "keyring";
    let resolved_token_type = resolve_token_type_for_debug(&profile_name, token_type)?;
    let endpoint = "https://slack.com/api/conversations.replies";

    debug::log_api_context(
        debug_level,
        Some(&profile_name),
        token_store_backend,
        resolved_token_type.as_str(),
        "conversations.replies",
        endpoint,
    );

    let client = get_api_client_with_token_type(Some(profile_name.clone()), token_type).await?;
    let output = build_thread_get_output(
        &client,
        channel,
        thread_ts,
        cmd.limit,
        inclusive,
        raw,
        profile_name,
        token_type,
    )
    .await?;

    debug::log_error_code(debug_level, &output);

    println!("{}", serde_json::to_string_pretty(&output).unwrap());
    Ok(())
}

pub async fn run_users_info(cmd: &args::UsersInfoArgs, globals: &GlobalArgs) -> Result<(), String> {
    let user = cmd.user_id.clone();
    let profile_name = globals.profile_name();
    let token_type = cmd.token_type;
    let raw = should_output_raw(cmd.raw);

    // Log debug information if --debug or --trace flag is present
    let debug_level = globals.debug_level;
    let token_store_backend = "keyring";
    let resolved_token_type = resolve_token_type_for_debug(&profile_name, token_type)?;
    let endpoint = "https://slack.com/api/users.info";

    debug::log_api_context(
        debug_level,
        Some(&profile_name),
        token_store_backend,
        resolved_token_type.as_str(),
        "users.info",
        endpoint,
    );

    let client = get_api_client_with_token_type(Some(profile_name.clone()), token_type).await?;
    let response = commands::users_info(&client, user)
        .await
        .map_err(|e| e.to_string())?;

    // Log error code if present
    debug::log_error_code(
        debug_level,
        &serde_json::to_value(&response).unwrap_or_default(),
    );

    // Display error guidance if response contains a known error
    crate::api::display_wrapper_error_guidance(&response);

    // Output with or without envelope
    let output = if raw {
        serde_json::to_string_pretty(&response).unwrap()
    } else {
        let response_value = serde_json::to_value(&response).map_err(|e| e.to_string())?;
        let wrapped = wrap_with_envelope_and_token_type(
            response_value,
            "users.info",
            "users info",
            Some(profile_name),
            token_type,
        )
        .await?;
        serde_json::to_string_pretty(&wrapped).unwrap()
    };

    println!("{}", output);
    Ok(())
}

pub async fn run_users_cache_update(
    cmd: &args::UsersCacheUpdateArgs,
    globals: &GlobalArgs,
) -> Result<(), String> {
    let profile_name = globals.profile_name();
    let token_type = cmd.token_type;

    let config_path = default_config_path().map_err(|e| e.to_string())?;
    let config = load_config(&config_path).map_err(|e| e.to_string())?;

    let profile = config
        .get(&profile_name)
        .ok_or_else(|| format!("Profile '{}' not found", profile_name))?;

    let client = get_api_client_with_token_type(Some(profile_name.clone()), token_type).await?;

    commands::update_cache(&client, profile.team_id.clone(), cmd.force)
        .await
        .map_err(|e| e.to_string())?;

    println!("Cache updated successfully for team {}", profile.team_id);
    Ok(())
}

pub async fn run_users_resolve_mentions(
    cmd: &args::UsersResolveMentionsArgs,
    globals: &GlobalArgs,
) -> Result<(), String> {
    let text = cmd.text.clone();
    let profile_name = globals.profile_name();
    let format_str = cmd
        .format
        .clone()
        .unwrap_or_else(|| "display_name".to_string());

    let format = format_str.parse::<commands::MentionFormat>().map_err(|_| {
        format!(
            "Invalid format: {}. Use display_name, real_name, or username",
            format_str
        )
    })?;

    let config_path = default_config_path().map_err(|e| e.to_string())?;
    let config = load_config(&config_path).map_err(|e| e.to_string())?;

    let profile = config
        .get(&profile_name)
        .ok_or_else(|| format!("Profile '{}' not found", profile_name))?;

    let cache_path = commands::UsersCacheFile::default_path()?;
    let cache_file = commands::UsersCacheFile::load(&cache_path)?;

    let workspace_cache = cache_file.get_workspace(&profile.team_id).ok_or_else(|| {
        format!(
            "No cache found for team {}. Run 'users cache-update' first.",
            profile.team_id
        )
    })?;

    let result = commands::resolve_mentions(&text, workspace_cache, format);
    println!("{}", result);
    Ok(())
}

/// Get team_id and user_id from profile
async fn get_team_and_user_ids_from_profile(
    profile_name: &str,
) -> Result<(String, String), String> {
    let config_path = default_config_path().map_err(|e| e.to_string())?;
    let profile = resolve_profile_full(&config_path, profile_name)
        .map_err(|e| format!("Failed to resolve profile '{}': {}", profile_name, e))?;
    Ok((profile.team_id, profile.user_id))
}

pub async fn run_msg_post(cmd: &args::MsgPostArgs, globals: &GlobalArgs) -> Result<(), String> {
    use crate::idempotency::{IdempotencyCheckResult, IdempotencyHandler};

    let non_interactive = globals.non_interactive;
    let channel = cmd.channel.clone();
    let text = cmd.text.clone();
    let thread_ts = cmd.thread_ts.clone();
    let reply_broadcast = cmd.reply_broadcast;
    let yes = cmd.yes;
    let profile_name = globals.profile_name();
    let token_type = cmd.token_type;
    let idempotency_key = cmd.idempotency_key.clone();

    // Validate: --reply-broadcast requires --thread-ts
    if reply_broadcast && thread_ts.is_none() {
        return Err("Error: --reply-broadcast requires --thread-ts".to_string());
    }

    let raw = should_output_raw(cmd.raw);
    let client = get_api_client_with_token_type(Some(profile_name.clone()), token_type).await?;

    // Check idempotency if key provided
    let (response_value, idempotency_status) = if let Some(key) = idempotency_key.clone() {
        let mut handler = IdempotencyHandler::new().map_err(|e| e.to_string())?;

        // Build params for fingerprinting
        let mut params = serde_json::Map::new();
        params.insert("channel".to_string(), serde_json::json!(channel.clone()));
        params.insert("text".to_string(), serde_json::json!(text.clone()));
        if let Some(ref ts) = thread_ts {
            params.insert("thread_ts".to_string(), serde_json::json!(ts));
            if reply_broadcast {
                params.insert("reply_broadcast".to_string(), serde_json::json!(true));
            }
        }

        // Get team_id and user_id from profile
        let (team_id, user_id) = get_team_and_user_ids_from_profile(&profile_name).await?;

        match handler
            .check(
                Some(key.clone()),
                team_id.clone(),
                user_id.clone(),
                "chat.postMessage".to_string(),
                &params,
            )
            .map_err(|e| e.to_string())?
        {
            IdempotencyCheckResult::Replay {
                response, status, ..
            } => {
                // Return cached response
                (response, Some(status))
            }
            IdempotencyCheckResult::Execute {
                key: scoped_key,
                fingerprint,
            } => {
                // Execute and store
                let response = commands::msg_post(
                    &client,
                    channel,
                    text,
                    thread_ts,
                    reply_broadcast,
                    yes,
                    non_interactive,
                )
                .await
                .map_err(|e| e.to_string())?;

                let response_value = serde_json::to_value(&response).map_err(|e| e.to_string())?;

                // Store result
                handler
                    .store(scoped_key, fingerprint, response_value.clone())
                    .map_err(|e| e.to_string())?;

                (
                    response_value,
                    Some(crate::idempotency::IdempotencyStatus::Executed),
                )
            }
            IdempotencyCheckResult::NoKey => unreachable!(),
        }
    } else {
        // No idempotency key - execute normally
        let response = commands::msg_post(
            &client,
            channel,
            text,
            thread_ts,
            reply_broadcast,
            yes,
            non_interactive,
        )
        .await
        .map_err(|e| e.to_string())?;

        (
            serde_json::to_value(&response).map_err(|e| e.to_string())?,
            None,
        )
    };

    // Display error guidance if response contains a known error
    if let Ok(api_response) =
        serde_json::from_value::<crate::api::ApiResponse>(response_value.clone())
    {
        crate::api::display_wrapper_error_guidance(&api_response);
    }

    // Output with or without envelope
    let output = if raw {
        serde_json::to_string_pretty(&response_value).unwrap()
    } else {
        let mut wrapped = wrap_with_envelope_and_token_type(
            response_value,
            "chat.postMessage",
            "msg post",
            Some(profile_name),
            token_type,
        )
        .await?;

        // Add idempotency metadata if key was provided
        if let (Some(key), Some(status)) = (idempotency_key, idempotency_status) {
            wrapped = wrapped.with_idempotency(
                key,
                match status {
                    crate::idempotency::IdempotencyStatus::Executed => "executed".to_string(),
                    crate::idempotency::IdempotencyStatus::Replayed => "replayed".to_string(),
                },
            );
        }

        serde_json::to_string_pretty(&wrapped).unwrap()
    };

    println!("{}", output);
    Ok(())
}

pub async fn run_msg_update(cmd: &args::MsgUpdateArgs, globals: &GlobalArgs) -> Result<(), String> {
    use crate::idempotency::{IdempotencyCheckResult, IdempotencyHandler};

    let non_interactive = globals.non_interactive;
    let channel = cmd.channel.clone();
    let ts = cmd.ts.clone();
    let text = cmd.text.clone();
    let yes = cmd.yes;
    let profile_name = globals.profile_name();
    let token_type = cmd.token_type;
    let idempotency_key = cmd.idempotency_key.clone();
    let raw = should_output_raw(cmd.raw);

    let client = get_api_client_with_token_type(Some(profile_name.clone()), token_type).await?;

    // Check idempotency if key provided
    let (response_value, idempotency_status) = if let Some(key) = idempotency_key.clone() {
        let mut handler = IdempotencyHandler::new().map_err(|e| e.to_string())?;

        let mut params = serde_json::Map::new();
        params.insert("channel".to_string(), serde_json::json!(channel.clone()));
        params.insert("ts".to_string(), serde_json::json!(ts.clone()));
        params.insert("text".to_string(), serde_json::json!(text.clone()));

        let (team_id, user_id) = get_team_and_user_ids_from_profile(&profile_name).await?;

        match handler
            .check(
                Some(key.clone()),
                team_id,
                user_id,
                "chat.update".to_string(),
                &params,
            )
            .map_err(|e| e.to_string())?
        {
            IdempotencyCheckResult::Replay {
                response, status, ..
            } => (response, Some(status)),
            IdempotencyCheckResult::Execute {
                key: scoped_key,
                fingerprint,
            } => {
                let response =
                    commands::msg_update(&client, channel, ts, text, yes, non_interactive)
                        .await
                        .map_err(|e| e.to_string())?;
                let response_value = serde_json::to_value(&response).map_err(|e| e.to_string())?;
                handler
                    .store(scoped_key, fingerprint, response_value.clone())
                    .map_err(|e| e.to_string())?;
                (
                    response_value,
                    Some(crate::idempotency::IdempotencyStatus::Executed),
                )
            }
            IdempotencyCheckResult::NoKey => unreachable!(),
        }
    } else {
        let response = commands::msg_update(&client, channel, ts, text, yes, non_interactive)
            .await
            .map_err(|e| e.to_string())?;
        (
            serde_json::to_value(&response).map_err(|e| e.to_string())?,
            None,
        )
    };

    if let Ok(api_response) =
        serde_json::from_value::<crate::api::ApiResponse>(response_value.clone())
    {
        crate::api::display_wrapper_error_guidance(&api_response);
    }

    let output = if raw {
        serde_json::to_string_pretty(&response_value).unwrap()
    } else {
        let mut wrapped = wrap_with_envelope_and_token_type(
            response_value,
            "chat.update",
            "msg update",
            Some(profile_name),
            token_type,
        )
        .await?;

        if let (Some(key), Some(status)) = (idempotency_key, idempotency_status) {
            wrapped = wrapped.with_idempotency(
                key,
                match status {
                    crate::idempotency::IdempotencyStatus::Executed => "executed".to_string(),
                    crate::idempotency::IdempotencyStatus::Replayed => "replayed".to_string(),
                },
            );
        }

        serde_json::to_string_pretty(&wrapped).unwrap()
    };

    println!("{}", output);
    Ok(())
}

pub async fn run_msg_delete(cmd: &args::MsgDeleteArgs, globals: &GlobalArgs) -> Result<(), String> {
    use crate::idempotency::{IdempotencyCheckResult, IdempotencyHandler};

    let non_interactive = globals.non_interactive;
    let channel = cmd.channel.clone();
    let ts = cmd.ts.clone();
    let yes = cmd.yes;
    let profile_name = globals.profile_name();
    let token_type = cmd.token_type;
    let idempotency_key = cmd.idempotency_key.clone();
    let raw = should_output_raw(cmd.raw);

    let client = get_api_client_with_token_type(Some(profile_name.clone()), token_type).await?;

    let (response_value, idempotency_status) = if let Some(key) = idempotency_key.clone() {
        let mut handler = IdempotencyHandler::new().map_err(|e| e.to_string())?;
        let mut params = serde_json::Map::new();
        params.insert("channel".to_string(), serde_json::json!(channel.clone()));
        params.insert("ts".to_string(), serde_json::json!(ts.clone()));
        let (team_id, user_id) = get_team_and_user_ids_from_profile(&profile_name).await?;
        match handler
            .check(
                Some(key.clone()),
                team_id,
                user_id,
                "chat.delete".to_string(),
                &params,
            )
            .map_err(|e| e.to_string())?
        {
            IdempotencyCheckResult::Replay {
                response, status, ..
            } => (response, Some(status)),
            IdempotencyCheckResult::Execute {
                key: scoped_key,
                fingerprint,
            } => {
                let response = commands::msg_delete(&client, channel, ts, yes, non_interactive)
                    .await
                    .map_err(|e| e.to_string())?;
                let response_value = serde_json::to_value(&response).map_err(|e| e.to_string())?;
                handler
                    .store(scoped_key, fingerprint, response_value.clone())
                    .map_err(|e| e.to_string())?;
                (
                    response_value,
                    Some(crate::idempotency::IdempotencyStatus::Executed),
                )
            }
            IdempotencyCheckResult::NoKey => unreachable!(),
        }
    } else {
        let response = commands::msg_delete(&client, channel, ts, yes, non_interactive)
            .await
            .map_err(|e| e.to_string())?;
        (
            serde_json::to_value(&response).map_err(|e| e.to_string())?,
            None,
        )
    };

    if let Ok(api_response) =
        serde_json::from_value::<crate::api::ApiResponse>(response_value.clone())
    {
        crate::api::display_wrapper_error_guidance(&api_response);
    }

    let output = if raw {
        serde_json::to_string_pretty(&response_value).unwrap()
    } else {
        let mut wrapped = wrap_with_envelope_and_token_type(
            response_value,
            "chat.delete",
            "msg delete",
            Some(profile_name),
            token_type,
        )
        .await?;
        if let (Some(key), Some(status)) = (idempotency_key, idempotency_status) {
            wrapped = wrapped.with_idempotency(
                key,
                match status {
                    crate::idempotency::IdempotencyStatus::Executed => "executed".to_string(),
                    crate::idempotency::IdempotencyStatus::Replayed => "replayed".to_string(),
                },
            );
        }
        serde_json::to_string_pretty(&wrapped).unwrap()
    };

    println!("{}", output);
    Ok(())
}

pub async fn run_react_add(cmd: &args::ReactArgs, globals: &GlobalArgs) -> Result<(), String> {
    use crate::idempotency::{IdempotencyCheckResult, IdempotencyHandler};

    let non_interactive = globals.non_interactive;
    let channel = cmd.channel.clone();
    let ts = cmd.ts.clone();
    let emoji = cmd.emoji.clone();
    let yes = cmd.yes;
    let profile_name = globals.profile_name();
    let token_type = cmd.token_type;
    let idempotency_key = cmd.idempotency_key.clone();
    let raw = should_output_raw(cmd.raw);

    let client = get_api_client_with_token_type(Some(profile_name.clone()), token_type).await?;

    let (response_value, idempotency_status) = if let Some(key) = idempotency_key.clone() {
        let mut handler = IdempotencyHandler::new().map_err(|e| e.to_string())?;
        let mut params = serde_json::Map::new();
        params.insert("channel".to_string(), serde_json::json!(channel.clone()));
        params.insert("timestamp".to_string(), serde_json::json!(ts.clone()));
        params.insert("name".to_string(), serde_json::json!(emoji.clone()));
        let (team_id, user_id) = get_team_and_user_ids_from_profile(&profile_name).await?;
        match handler
            .check(
                Some(key.clone()),
                team_id,
                user_id,
                "reactions.add".to_string(),
                &params,
            )
            .map_err(|e| e.to_string())?
        {
            IdempotencyCheckResult::Replay {
                response, status, ..
            } => (response, Some(status)),
            IdempotencyCheckResult::Execute {
                key: scoped_key,
                fingerprint,
            } => {
                let response =
                    commands::react_add(&client, channel, ts, emoji, yes, non_interactive)
                        .await
                        .map_err(|e| e.to_string())?;
                let response_value = serde_json::to_value(&response).map_err(|e| e.to_string())?;
                handler
                    .store(scoped_key, fingerprint, response_value.clone())
                    .map_err(|e| e.to_string())?;
                (
                    response_value,
                    Some(crate::idempotency::IdempotencyStatus::Executed),
                )
            }
            IdempotencyCheckResult::NoKey => unreachable!(),
        }
    } else {
        let response = commands::react_add(&client, channel, ts, emoji, yes, non_interactive)
            .await
            .map_err(|e| e.to_string())?;
        (
            serde_json::to_value(&response).map_err(|e| e.to_string())?,
            None,
        )
    };

    if let Ok(api_response) =
        serde_json::from_value::<crate::api::ApiResponse>(response_value.clone())
    {
        crate::api::display_wrapper_error_guidance(&api_response);
    }

    let output = if raw {
        serde_json::to_string_pretty(&response_value).unwrap()
    } else {
        let mut wrapped = wrap_with_envelope_and_token_type(
            response_value,
            "reactions.add",
            "react add",
            Some(profile_name),
            token_type,
        )
        .await?;
        if let (Some(key), Some(status)) = (idempotency_key, idempotency_status) {
            wrapped = wrapped.with_idempotency(
                key,
                match status {
                    crate::idempotency::IdempotencyStatus::Executed => "executed".to_string(),
                    crate::idempotency::IdempotencyStatus::Replayed => "replayed".to_string(),
                },
            );
        }
        serde_json::to_string_pretty(&wrapped).unwrap()
    };

    println!("{}", output);
    Ok(())
}

pub async fn run_react_remove(cmd: &args::ReactArgs, globals: &GlobalArgs) -> Result<(), String> {
    use crate::idempotency::{IdempotencyCheckResult, IdempotencyHandler};

    let non_interactive = globals.non_interactive;
    let channel = cmd.channel.clone();
    let ts = cmd.ts.clone();
    let emoji = cmd.emoji.clone();
    let yes = cmd.yes;
    let profile_name = globals.profile_name();
    let token_type = cmd.token_type;
    let idempotency_key = cmd.idempotency_key.clone();
    let raw = should_output_raw(cmd.raw);

    let client = get_api_client_with_token_type(Some(profile_name.clone()), token_type).await?;

    let (response_value, idempotency_status) = if let Some(key) = idempotency_key.clone() {
        let mut handler = IdempotencyHandler::new().map_err(|e| e.to_string())?;
        let mut params = serde_json::Map::new();
        params.insert("channel".to_string(), serde_json::json!(channel.clone()));
        params.insert("timestamp".to_string(), serde_json::json!(ts.clone()));
        params.insert("name".to_string(), serde_json::json!(emoji.clone()));
        let (team_id, user_id) = get_team_and_user_ids_from_profile(&profile_name).await?;
        match handler
            .check(
                Some(key.clone()),
                team_id,
                user_id,
                "reactions.remove".to_string(),
                &params,
            )
            .map_err(|e| e.to_string())?
        {
            IdempotencyCheckResult::Replay {
                response, status, ..
            } => (response, Some(status)),
            IdempotencyCheckResult::Execute {
                key: scoped_key,
                fingerprint,
            } => {
                let response =
                    commands::react_remove(&client, channel, ts, emoji, yes, non_interactive)
                        .await
                        .map_err(|e| e.to_string())?;
                let response_value = serde_json::to_value(&response).map_err(|e| e.to_string())?;
                handler
                    .store(scoped_key, fingerprint, response_value.clone())
                    .map_err(|e| e.to_string())?;
                (
                    response_value,
                    Some(crate::idempotency::IdempotencyStatus::Executed),
                )
            }
            IdempotencyCheckResult::NoKey => unreachable!(),
        }
    } else {
        let response = commands::react_remove(&client, channel, ts, emoji, yes, non_interactive)
            .await
            .map_err(|e| e.to_string())?;
        (
            serde_json::to_value(&response).map_err(|e| e.to_string())?,
            None,
        )
    };

    if let Ok(api_response) =
        serde_json::from_value::<crate::api::ApiResponse>(response_value.clone())
    {
        crate::api::display_wrapper_error_guidance(&api_response);
    }

    let output = if raw {
        serde_json::to_string_pretty(&response_value).unwrap()
    } else {
        let mut wrapped = wrap_with_envelope_and_token_type(
            response_value,
            "reactions.remove",
            "react remove",
            Some(profile_name),
            token_type,
        )
        .await?;
        if let (Some(key), Some(status)) = (idempotency_key, idempotency_status) {
            wrapped = wrapped.with_idempotency(
                key,
                match status {
                    crate::idempotency::IdempotencyStatus::Executed => "executed".to_string(),
                    crate::idempotency::IdempotencyStatus::Replayed => "replayed".to_string(),
                },
            );
        }
        serde_json::to_string_pretty(&wrapped).unwrap()
    };

    println!("{}", output);
    Ok(())
}

pub async fn run_file_upload(
    cmd: &args::FileUploadArgs,
    globals: &GlobalArgs,
) -> Result<(), String> {
    use crate::idempotency::{IdempotencyCheckResult, IdempotencyHandler};

    let non_interactive = globals.non_interactive;
    let file_path = cmd.path.clone();
    let channels = cmd.channel.clone().or_else(|| cmd.channels.clone());
    let title = cmd.title.clone();
    let comment = cmd.comment.clone();
    let yes = cmd.yes;
    let profile_name = globals.profile_name();
    let token_type = cmd.token_type;
    let idempotency_key = cmd.idempotency_key.clone();
    let raw = should_output_raw(cmd.raw);

    let client = get_api_client_with_token_type(Some(profile_name.clone()), token_type).await?;

    let (response_value, idempotency_status) = if let Some(key) = idempotency_key.clone() {
        let mut handler = IdempotencyHandler::new().map_err(|e| e.to_string())?;
        let mut params = serde_json::Map::new();
        params.insert("filename".to_string(), serde_json::json!(file_path.clone()));
        if let Some(ref ch) = channels {
            params.insert("channels".to_string(), serde_json::json!(ch));
        }
        if let Some(ref t) = title {
            params.insert("title".to_string(), serde_json::json!(t));
        }
        if let Some(ref c) = comment {
            params.insert("comment".to_string(), serde_json::json!(c));
        }
        let (team_id, user_id) = get_team_and_user_ids_from_profile(&profile_name).await?;
        match handler
            .check(
                Some(key.clone()),
                team_id,
                user_id,
                "files.upload".to_string(),
                &params,
            )
            .map_err(|e| e.to_string())?
        {
            IdempotencyCheckResult::Replay {
                response, status, ..
            } => (response, Some(status)),
            IdempotencyCheckResult::Execute {
                key: scoped_key,
                fingerprint,
            } => {
                let response = commands::file_upload(
                    &client,
                    file_path,
                    channels,
                    title,
                    comment,
                    yes,
                    non_interactive,
                )
                .await
                .map_err(|e| e.to_string())?;
                let response_value = serde_json::to_value(&response).map_err(|e| e.to_string())?;
                handler
                    .store(scoped_key, fingerprint, response_value.clone())
                    .map_err(|e| e.to_string())?;
                (
                    response_value,
                    Some(crate::idempotency::IdempotencyStatus::Executed),
                )
            }
            IdempotencyCheckResult::NoKey => unreachable!(),
        }
    } else {
        let response = commands::file_upload(
            &client,
            file_path,
            channels,
            title,
            comment,
            yes,
            non_interactive,
        )
        .await
        .map_err(|e| e.to_string())?;
        (
            serde_json::to_value(&response).map_err(|e| e.to_string())?,
            None,
        )
    };

    crate::api::display_json_error_guidance(&response_value);

    let output = if raw {
        serde_json::to_string_pretty(&response_value).unwrap()
    } else {
        let mut wrapped = wrap_with_envelope_and_token_type(
            response_value,
            "files.upload",
            "file upload",
            Some(profile_name),
            token_type,
        )
        .await?;
        if let (Some(key), Some(status)) = (idempotency_key, idempotency_status) {
            wrapped = wrapped.with_idempotency(
                key,
                match status {
                    crate::idempotency::IdempotencyStatus::Executed => "executed".to_string(),
                    crate::idempotency::IdempotencyStatus::Replayed => "replayed".to_string(),
                },
            );
        }
        serde_json::to_string_pretty(&wrapped).unwrap()
    };

    println!("{}", output);
    Ok(())
}

pub async fn run_file_download(
    cmd: &args::FileDownloadArgs,
    globals: &GlobalArgs,
) -> Result<(), String> {
    let file_id = cmd.file_id.clone();
    let url = cmd.url.clone();
    let out = cmd.out.clone();
    let profile_name = globals.profile_name();
    let token_type = cmd.token_type;
    let raw = should_output_raw(cmd.raw);

    // Validate: at least one of file_id or url must be provided
    if file_id.is_none() && url.is_none() {
        return Err("Either <file_id> or --url must be provided".to_string());
    }

    let client = get_api_client_with_token_type(Some(profile_name.clone()), token_type).await?;
    let response = commands::file_download(&client, file_id, url, out)
        .await
        .map_err(|e| e.to_string())?;

    // For --out -, don't print any output (file bytes already written to stdout)
    if let Some(out_path) = response.get("output").and_then(|v| v.as_str()) {
        if out_path == "-" {
            return Ok(());
        }
    }

    // Display error guidance if response contains a known error
    crate::api::display_json_error_guidance(&response);

    // Output with or without envelope
    let output = if raw {
        serde_json::to_string_pretty(&response).unwrap()
    } else {
        let wrapped = wrap_with_envelope_and_token_type(
            response,
            "files.info + download",
            "file download",
            Some(profile_name),
            token_type,
        )
        .await?;
        serde_json::to_string_pretty(&wrapped).unwrap()
    };

    println!("{}", output);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    // Mock token store for testing
    struct MockTokenStore {
        tokens: std::collections::HashMap<String, String>,
    }

    impl MockTokenStore {
        fn new() -> Self {
            Self {
                tokens: std::collections::HashMap::new(),
            }
        }

        fn with_token(mut self, key: &str, value: &str) -> Self {
            self.tokens.insert(key.to_string(), value.to_string());
            self
        }
    }

    impl TokenStore for MockTokenStore {
        fn get(&self, key: &str) -> crate::profile::token_store::Result<String> {
            use crate::profile::token_store::TokenStoreError;
            self.tokens
                .get(key)
                .cloned()
                .ok_or_else(|| TokenStoreError::NotFound(key.to_string()))
        }

        fn set(&self, _key: &str, _value: &str) -> crate::profile::token_store::Result<()> {
            unimplemented!("set not needed for tests")
        }

        fn delete(&self, _key: &str) -> crate::profile::token_store::Result<()> {
            unimplemented!("delete not needed for tests")
        }

        fn exists(&self, key: &str) -> bool {
            self.tokens.contains_key(key)
        }
    }

    #[test]
    fn test_resolve_token_uses_store() {
        let store = MockTokenStore::new().with_token("T123:U123", "xoxb-store-token");

        let result = resolve_token_for_wrapper(&store, "T123:U123", None, false);

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "xoxb-store-token");
    }

    #[test]
    fn test_resolve_token_explicit_request() {
        // When token type is explicitly requested, don't fallback
        let store = MockTokenStore::new().with_token("T123:U123", "xoxb-bot-token");

        let result = resolve_token_for_wrapper(
            &store,
            "T123:U123:user",  // User token key
            Some("T123:U123"), // Bot token fallback
            true,              // Explicit request
        );

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("explicitly requested"));
    }

    #[test]
    fn test_resolve_token_fallback_when_not_explicit() {
        // When token type is not explicitly requested, allow fallback
        let store = MockTokenStore::new().with_token("T123:U123", "xoxb-bot-token");

        let result = resolve_token_for_wrapper(
            &store,
            "T123:U123:user",  // User token key (not found)
            Some("T123:U123"), // Bot token fallback
            false,             // Not explicit request
        );

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "xoxb-bot-token");
    }

    #[test]
    fn test_resolve_token_error_mentions_login() {
        let store = MockTokenStore::new();

        let result = resolve_token_for_wrapper(&store, "T123:U123", None, false);

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("slack auth login"));
    }

    // Tests for get_option with space-separated format (used by `api call` rest parsing)
    #[test]
    fn test_get_option_equals_format() {
        let args = vec!["cmd".to_string(), "--filter=is_private:true".to_string()];
        assert_eq!(
            get_option(&args, "--filter="),
            Some("is_private:true".to_string())
        );
    }

    #[test]
    fn test_get_option_space_separated() {
        let args = vec![
            "cmd".to_string(),
            "--filter".to_string(),
            "is_private:true".to_string(),
        ];
        assert_eq!(
            get_option(&args, "--filter="),
            Some("is_private:true".to_string())
        );
    }

    #[test]
    fn test_get_option_space_separated_rejects_dash_value() {
        // Value starting with '-' should not be treated as value
        let args = vec![
            "cmd".to_string(),
            "--filter".to_string(),
            "--other".to_string(),
        ];
        assert_eq!(get_option(&args, "--filter="), None);
    }

    #[test]
    fn test_get_option_space_separated_missing_value() {
        let args = vec!["cmd".to_string(), "--filter".to_string()];
        assert_eq!(get_option(&args, "--filter="), None);
    }

    #[test]
    fn test_get_option_prefers_equals_format() {
        // When both formats exist, equals format should be returned first
        let args = vec![
            "--filter=value1".to_string(),
            "--filter".to_string(),
            "value2".to_string(),
        ];
        assert_eq!(get_option(&args, "--filter="), Some("value1".to_string()));
    }

    #[test]
    fn test_get_all_options_mixed_format() {
        let args = vec![
            "cmd".to_string(),
            "--filter=is_private:true".to_string(),
            "--filter".to_string(),
            "is_member:true".to_string(),
            "--filter=name:test".to_string(),
            "--filter".to_string(),
            "is_archived:false".to_string(),
        ];
        let result = get_all_options(&args, "--filter=");
        assert_eq!(
            result,
            vec![
                "is_private:true",
                "name:test",
                "is_member:true",
                "is_archived:false"
            ]
        );
    }

    // Tests for resolve_profile_name (used by `api call` rest parsing)
    #[test]
    fn test_resolve_profile_name_with_equals_format() {
        let args = vec![
            "slack".to_string(),
            "api".to_string(),
            "call".to_string(),
            "--profile=myprofile".to_string(),
            "test.method".to_string(),
        ];
        assert_eq!(resolve_profile_name(&args), "myprofile");
    }

    #[test]
    fn test_resolve_profile_name_with_space_format() {
        let args = vec![
            "slack".to_string(),
            "api".to_string(),
            "call".to_string(),
            "--profile".to_string(),
            "myprofile".to_string(),
            "test.method".to_string(),
        ];
        assert_eq!(resolve_profile_name(&args), "myprofile");
    }

    #[test]
    #[serial]
    fn test_resolve_profile_name_env_fallback() {
        // Set environment variable
        std::env::set_var("SLACK_PROFILE", "envprofile");

        let args = vec!["slack".to_string(), "api".to_string(), "call".to_string()];
        assert_eq!(resolve_profile_name(&args), "envprofile");

        // Clean up
        std::env::remove_var("SLACK_PROFILE");
    }

    #[test]
    #[serial]
    fn test_resolve_profile_name_default_fallback() {
        // Ensure SLACK_PROFILE is not set
        std::env::remove_var("SLACK_PROFILE");

        let args = vec!["slack".to_string(), "api".to_string(), "call".to_string()];
        assert_eq!(resolve_profile_name(&args), "default");
    }

    #[test]
    #[serial]
    fn test_resolve_profile_name_flag_overrides_env() {
        // Set environment variable
        std::env::set_var("SLACK_PROFILE", "envprofile");

        let args = vec![
            "slack".to_string(),
            "api".to_string(),
            "--profile=flagprofile".to_string(),
            "call".to_string(),
        ];
        assert_eq!(resolve_profile_name(&args), "flagprofile");

        // Clean up
        std::env::remove_var("SLACK_PROFILE");
    }

    // Tests for GlobalArgs::profile_name resolution
    #[test]
    #[serial]
    fn test_global_args_profile_flag_wins() {
        std::env::set_var("SLACK_PROFILE", "envprofile");
        let globals = GlobalArgs {
            profile: Some("flagprofile".to_string()),
            non_interactive: false,
            debug_level: debug::DebugLevel::Off,
        };
        assert_eq!(globals.profile_name(), "flagprofile");
        std::env::remove_var("SLACK_PROFILE");
    }

    #[test]
    #[serial]
    fn test_global_args_profile_env_fallback() {
        std::env::set_var("SLACK_PROFILE", "envprofile");
        let globals = GlobalArgs {
            profile: None,
            non_interactive: false,
            debug_level: debug::DebugLevel::Off,
        };
        assert_eq!(globals.profile_name(), "envprofile");
        std::env::remove_var("SLACK_PROFILE");
    }

    #[test]
    #[serial]
    fn test_global_args_profile_default_fallback() {
        std::env::remove_var("SLACK_PROFILE");
        let globals = GlobalArgs {
            profile: None,
            non_interactive: false,
            debug_level: debug::DebugLevel::Off,
        };
        assert_eq!(globals.profile_name(), "default");
    }

    #[test]
    #[serial]
    fn test_should_output_raw_flag_and_env() {
        std::env::remove_var("SLACKRS_OUTPUT");
        assert!(should_output_raw(true));
        assert!(!should_output_raw(false));

        std::env::set_var("SLACKRS_OUTPUT", "raw");
        assert!(should_output_raw(false));

        std::env::set_var("SLACKRS_OUTPUT", "envelope");
        assert!(!should_output_raw(false));
        assert!(should_output_raw(true));

        std::env::remove_var("SLACKRS_OUTPUT");
    }
}
