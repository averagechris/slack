// Use library exports instead of module declarations to avoid duplicate test runs
use clap::{CommandFactory, Parser};
use slack::cli::args::{
    ApiCommand, AuthCommand, Cli, Command, ConfigCommand, ConfigOauthCommand, ConvCommand,
    FileCommand, MsgCommand, ReactCommand, ThreadCommand, UsersCommand,
};
use slack::debug::DebugLevel;
use slack::{auth, cli, commands, profile};

#[tokio::main]
async fn main() {
    let raw_args: Vec<String> = std::env::args().collect();

    // Early check for `--help --json` (applies to all commands). This is
    // intercepted before clap because clap owns --help and would print the
    // human-readable help instead. The JSON is generated from the clap model.
    if raw_args.iter().any(|a| a == "--help") && raw_args.iter().any(|a| a == "--json") {
        handle_help_json(&raw_args);
        return;
    }

    // Parse with clap. Exit-code contract (preserved from the pre-clap CLI):
    // help/version exit 0; usage errors exit 1 (clap's default of 2 is
    // reserved for "interactive input required in non-interactive mode").
    let cli_args = match Cli::try_parse_from(&raw_args) {
        Ok(cli_args) => cli_args,
        Err(e) => {
            use clap::error::ErrorKind;
            let _ = e.print();
            match e.kind() {
                ErrorKind::DisplayHelp
                | ErrorKind::DisplayVersion
                | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand => std::process::exit(0),
                _ => std::process::exit(1),
            }
        }
    };

    let ctx = cli::CliContext::new(cli_args.non_interactive);
    let debug_level = if cli_args.trace {
        DebugLevel::Trace
    } else if cli_args.debug || slack::debug::enabled() {
        DebugLevel::Debug
    } else {
        DebugLevel::Off
    };

    let globals = cli::GlobalArgs {
        profile: cli_args.profile.clone(),
        non_interactive: ctx.is_non_interactive(),
        debug_level,
    };

    match &cli_args.command {
        Command::Api { command } => match command {
            ApiCommand::Call(call_args) => {
                // `api call` keeps free-form parsing over the trailing args;
                // pass through global flags so --profile/--debug/--trace work
                // in any position.
                let mut rest = call_args.rest.clone();
                if let Some(profile) = &globals.profile {
                    if cli::get_option(&rest, "--profile=").is_none() {
                        rest.push(format!("--profile={}", profile));
                    }
                }
                if globals.debug_level >= DebugLevel::Trace && !cli::has_flag(&rest, "--trace") {
                    rest.push("--trace".to_string());
                } else if globals.debug_level >= DebugLevel::Debug
                    && !cli::has_flag(&rest, "--debug")
                {
                    rest.push("--debug".to_string());
                }
                if let Err(e) = cli::run_api_call(rest).await {
                    handle_command_error(&e.to_string(), "Error");
                }
            }
        },
        Command::Auth { command } => handle_auth_command(command, &globals, &ctx).await,
        Command::Config { command } => handle_config_command(command),
        Command::Search(search_args) => {
            if let Err(e) = cli::run_search(search_args, &globals).await {
                handle_command_error(&e.to_string(), "Search failed");
            }
        }
        Command::Conv { command } => match command {
            ConvCommand::List(list_args) => {
                if let Err(e) = cli::run_conv_list(list_args, &globals).await {
                    handle_command_error(&e.to_string(), "Conv list failed");
                }
            }
            ConvCommand::Select(select_args) => {
                if let Err(e) = cli::run_conv_select(select_args, &globals).await {
                    handle_command_error(&e.to_string(), "Conv select failed");
                }
            }
            ConvCommand::Search(search_args) => {
                if let Err(e) = cli::run_conv_search(search_args, &globals).await {
                    handle_command_error(&e.to_string(), "Conv search failed");
                }
            }
            ConvCommand::History(history_args) => {
                if let Err(e) = cli::run_conv_history(history_args, &globals).await {
                    handle_command_error(&e.to_string(), "Conv history failed");
                }
            }
            ConvCommand::Open(open_args) => {
                if let Err(e) = cli::run_conv_open(open_args, &globals).await {
                    handle_command_error(&e.to_string(), "Conv open failed");
                }
            }
        },
        Command::Thread { command } => match command {
            ThreadCommand::Get(get_args) => {
                if let Err(e) = cli::run_thread_get(get_args, &globals).await {
                    handle_command_error(&e.to_string(), "Thread get failed");
                }
            }
        },
        Command::Users { command } => match command {
            UsersCommand::Info(info_args) => {
                if let Err(e) = cli::run_users_info(info_args, &globals).await {
                    handle_command_error(&e.to_string(), "Users info failed");
                }
            }
            UsersCommand::Lookup(lookup_args) => {
                if let Err(e) = cli::run_users_lookup(lookup_args, &globals).await {
                    handle_command_error(&e.to_string(), "Users lookup failed");
                }
            }
            UsersCommand::CacheUpdate(update_args) => {
                if let Err(e) = cli::run_users_cache_update(update_args, &globals).await {
                    handle_command_error(&e.to_string(), "Users cache-update failed");
                }
            }
            UsersCommand::ResolveMentions(resolve_args) => {
                if let Err(e) = cli::run_users_resolve_mentions(resolve_args, &globals).await {
                    handle_command_error(&e.to_string(), "Users resolve-mentions failed");
                }
            }
        },
        Command::Msg { command } => match command {
            MsgCommand::Post(post_args) => {
                if let Err(e) = cli::run_msg_post(post_args, &globals).await {
                    handle_command_error(&e.to_string(), "Msg post failed");
                }
            }
            MsgCommand::Update(update_args) => {
                if let Err(e) = cli::run_msg_update(update_args, &globals).await {
                    handle_command_error(&e.to_string(), "Msg update failed");
                }
            }
            MsgCommand::Delete(delete_args) => {
                if let Err(e) = cli::run_msg_delete(delete_args, &globals).await {
                    handle_command_error(&e.to_string(), "Msg delete failed");
                }
            }
        },
        Command::React { command } => match command {
            ReactCommand::Add(react_args) => {
                if let Err(e) = cli::run_react_add(react_args, &globals).await {
                    handle_command_error(&e.to_string(), "React add failed");
                }
            }
            ReactCommand::Remove(react_args) => {
                if let Err(e) = cli::run_react_remove(react_args, &globals).await {
                    handle_command_error(&e.to_string(), "React remove failed");
                }
            }
        },
        Command::File { command } => match command {
            FileCommand::Upload(upload_args) => {
                if let Err(e) = cli::run_file_upload(upload_args, &globals).await {
                    handle_command_error(&e.to_string(), "File upload failed");
                }
            }
            FileCommand::Download(download_args) => {
                if let Err(e) = cli::run_file_download(download_args, &globals).await {
                    handle_command_error(&e.to_string(), "File download failed");
                }
            }
        },
        Command::Commands { json } => {
            if *json {
                let response = cli::generate_commands_list();
                let json = serde_json::to_string_pretty(&response).unwrap();
                println!("{}", json);
            } else {
                eprintln!("Usage: slack commands --json");
                std::process::exit(1);
            }
        }
        Command::Schema { command, output } => match (command, output) {
            (Some(cmd), Some(out)) => {
                if out == "json-schema" {
                    match cli::generate_schema(cmd) {
                        Ok(schema_response) => {
                            let json = serde_json::to_string_pretty(&schema_response).unwrap();
                            println!("{}", json);
                        }
                        Err(e) => {
                            handle_command_error(&e, "Schema error");
                        }
                    }
                } else {
                    eprintln!("Invalid output format. Use --output json-schema");
                    std::process::exit(1);
                }
            }
            _ => {
                eprintln!("Usage: slack schema --command <cmd> --output json-schema");
                std::process::exit(1);
            }
        },
        Command::Doctor(doctor_args) => {
            if let Err(e) = commands::doctor(globals.profile.clone(), doctor_args.json) {
                handle_command_error(&e.to_string(), "Doctor command failed");
            }
        }
        Command::InstallSkills(install_args) => {
            if let Err(e) = cli::run_install_skill(install_args) {
                handle_command_error(&e, "Skill installation failed");
            }
        }
        Command::Completions { shell } => {
            let mut cmd = Cli::command();
            clap_complete::generate(*shell, &mut cmd, "slack", &mut std::io::stdout());
        }
    }
}

/// Handle `--help --json` (structured help generated from the clap model).
///
/// With a command path (e.g. `slack conv list --help --json`) this prints
/// structured help for that command; without one (`slack --help --json`) it
/// prints the full commands list.
fn handle_help_json(raw_args: &[String]) {
    let command_parts: Vec<String> = raw_args[1..]
        .iter()
        .filter(|arg| !arg.starts_with("--") && !arg.starts_with('-'))
        .map(|s| s.to_string())
        .collect();

    if command_parts.is_empty() {
        let response = cli::generate_commands_list();
        let json = serde_json::to_string_pretty(&response).unwrap();
        println!("{}", json);
        return;
    }

    let command_name = command_parts.join(" ");
    match cli::generate_help(&command_name) {
        Ok(help) => {
            let json = serde_json::to_string_pretty(&help).unwrap();
            println!("{}", json);
        }
        Err(e) => {
            eprintln!("Help generation failed: {}", e);
            std::process::exit(1);
        }
    }
}

/// Handle command error and exit with appropriate code
///
/// This helper consolidates the common error handling pattern:
/// - Print error message to stderr with prefix
/// - Exit with code 2 for non-interactive errors, code 1 otherwise
fn handle_command_error(error: &str, prefix: &str) -> ! {
    eprintln!("{}: {}", prefix, error);

    // Check if this is a non-interactive error
    if cli::is_non_interactive_error(error) {
        std::process::exit(2);
    }
    std::process::exit(1);
}

/// Handle auth subcommand dispatch
async fn handle_auth_command(
    command: &AuthCommand,
    globals: &cli::GlobalArgs,
    ctx: &cli::CliContext,
) {
    match command {
        AuthCommand::Login(login_args) => {
            if let Err(e) = cli::run_auth_login(login_args, ctx.is_non_interactive()).await {
                handle_command_error(&e.to_string(), "Login failed");
            }
        }
        AuthCommand::Status { profile_name } => {
            if let Err(e) = auth::status(profile_name.clone()) {
                handle_command_error(&e.to_string(), "Status command failed");
            }
        }
        AuthCommand::List => {
            if let Err(e) = auth::list() {
                handle_command_error(&e.to_string(), "List command failed");
            }
        }
        AuthCommand::Rename { old_name, new_name } => {
            if let Err(e) = auth::rename(old_name.clone(), new_name.clone()) {
                handle_command_error(&e.to_string(), "Rename command failed");
            }
        }
        AuthCommand::Logout { profile_name } => {
            if let Err(e) = auth::logout(profile_name.clone()) {
                handle_command_error(&e.to_string(), "Logout command failed");
            }
        }
        AuthCommand::Export(export_args) => {
            cli::handle_export_command(export_args, globals.profile.clone()).await;
        }
        AuthCommand::Import(import_args) => {
            cli::handle_import_command(import_args).await;
        }
        AuthCommand::Migrate { path } => {
            if let Err(e) = auth::migrate(path.clone()) {
                handle_command_error(&e.to_string(), "Migrate command failed");
            }
        }
    }
}

/// Handle config subcommand dispatch
fn handle_config_command(command: &ConfigCommand) {
    match command {
        ConfigCommand::Oauth { command } => match command {
            ConfigOauthCommand::Set(set_args) => {
                if let Err(e) = run_config_oauth_set(set_args) {
                    handle_command_error(&e, "OAuth config set failed");
                }
            }
            ConfigOauthCommand::Show { profile_name } => {
                if let Err(e) =
                    commands::oauth_show(profile_name.clone()).map_err(|e| e.to_string())
                {
                    handle_command_error(&e, "OAuth config show failed");
                }
            }
            ConfigOauthCommand::Delete { profile_name } => {
                if let Err(e) =
                    commands::oauth_delete(profile_name.clone()).map_err(|e| e.to_string())
                {
                    handle_command_error(&e, "OAuth config delete failed");
                }
            }
        },
        ConfigCommand::Set {
            profile_name,
            token_type,
        } => {
            if let Err(e) = run_config_set(profile_name, *token_type) {
                handle_command_error(&e, "Config set failed");
            }
        }
    }
}

/// Run config oauth set command
fn run_config_oauth_set(args: &slack::cli::args::ConfigOauthSetArgs) -> Result<(), String> {
    if args.client_secret.is_some() {
        return Err(
            "--client-secret was removed for security (secrets land in shell history).\n\
             Provide the secret via:\n\
             - Environment variable: SLACKRS_CLIENT_SECRET=<secret>\n\
             - Flag: --client-secret-env <ENV_VAR>\n\
             - Flag: --client-secret-file <PATH>\n\
             - Interactive prompt (run in a terminal)"
                .to_string(),
        );
    }

    let client = args
        .client_id
        .clone()
        .ok_or_else(|| "--client-id is required".to_string())?;
    let redirect = args
        .redirect_uri
        .clone()
        .ok_or_else(|| "--redirect-uri is required".to_string())?;
    let scope_str = args
        .scopes
        .clone()
        .ok_or_else(|| "--scopes is required".to_string())?;

    commands::oauth_set(commands::OAuthSetParams {
        profile_name: args.profile_name.clone(),
        client_id: client,
        redirect_uri: redirect,
        scopes: scope_str,
        client_secret_env: args.client_secret_env.clone(),
        client_secret_file: args.client_secret_file.clone(),
    })
    .map_err(|e| e.to_string())
}

/// Run config set command
fn run_config_set(
    profile_name: &str,
    token_type: Option<profile::TokenType>,
) -> Result<(), String> {
    let ttype = token_type.ok_or_else(|| "--token-type is required".to_string())?;
    commands::set_default_token_type(profile_name.to_string(), ttype).map_err(|e| e.to_string())
}
