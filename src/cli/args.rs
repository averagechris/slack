//! clap-based CLI argument definitions (D3: clap migration).
//!
//! This module is the single source of truth for the command surface. The
//! introspection commands (`commands --json`, `schema`, `--help --json`) are
//! generated from this model at runtime — see [`crate::cli::introspect`].

use crate::profile::TokenType;
use clap::{ArgAction, Args, Parser, Subcommand};
use clap_complete::Shell;

/// Parse a `--token-type` value (bot or user).
fn token_type_value(s: &str) -> Result<TokenType, String> {
    s.parse::<TokenType>().map_err(|e| e.to_string())
}

/// Top-level CLI definition.
#[derive(Debug, Parser)]
#[command(
    name = "slack",
    bin_name = "slack",
    version,
    about = "Slack CLI with OAuth authentication, profile management, and API access",
    disable_version_flag = true,
    subcommand_required = true,
    arg_required_else_help = true
)]
pub struct Cli {
    /// Print version
    #[arg(short = 'v', long = "version", action = ArgAction::Version)]
    version: Option<bool>,

    /// Profile to use (falls back to SLACK_PROFILE env, then 'default')
    #[arg(long, global = true, value_name = "NAME")]
    pub profile: Option<String>,

    /// Run without interactive prompts (auto-enabled when stdin is not a TTY)
    #[arg(long, global = true)]
    pub non_interactive: bool,

    /// Show debug information (profile, token type, API method)
    #[arg(long, global = true)]
    pub debug: bool,

    /// Show verbose trace information
    #[arg(long, global = true)]
    pub trace: bool,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Call Slack Web API methods
    Api {
        #[command(subcommand)]
        command: ApiCommand,
    },
    /// Authentication and profile management
    Auth {
        #[command(subcommand)]
        command: AuthCommand,
    },
    /// Profile configuration
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    /// Search messages
    Search(SearchArgs),
    /// Conversation (channel) commands
    Conv {
        #[command(subcommand)]
        command: ConvCommand,
    },
    /// Thread commands
    Thread {
        #[command(subcommand)]
        command: ThreadCommand,
    },
    /// User commands
    Users {
        #[command(subcommand)]
        command: UsersCommand,
    },
    /// Message write commands (requires SLACKCLI_ALLOW_WRITE=true)
    Msg {
        #[command(subcommand)]
        command: MsgCommand,
    },
    /// Reaction write commands (requires SLACKCLI_ALLOW_WRITE=true)
    React {
        #[command(subcommand)]
        command: ReactCommand,
    },
    /// File upload/download commands
    File {
        #[command(subcommand)]
        command: FileCommand,
    },
    /// List all available commands in machine-readable format
    Commands {
        /// Output in JSON format (required)
        #[arg(long)]
        json: bool,
    },
    /// Show output schema for a command
    Schema {
        /// Command name (e.g. 'conv list' or 'conv.list')
        #[arg(long, value_name = "CMD")]
        command: Option<String>,
        /// Output format (json-schema)
        #[arg(long, value_name = "FORMAT")]
        output: Option<String>,
    },
    /// Show diagnostic information about the CLI environment
    Doctor(DoctorArgs),
    /// Install agent skill (default source: self)
    InstallSkills(InstallSkillsArgs),
    /// Generate shell completions
    Completions {
        /// Shell to generate completions for
        #[arg(value_enum)]
        shell: Shell,
    },
}

#[derive(Debug, Subcommand)]
pub enum ApiCommand {
    /// Call a Slack API method: api call <method> [key=value]...
    ///
    /// Free-form parameters are passed as key=value pairs. Supported flags:
    /// --json (send JSON body), --get (use GET), --raw (raw Slack response),
    /// --token-type bot|user. Unknown flags are ignored for forward
    /// compatibility.
    Call(ApiCallCliArgs),
}

/// `api call` keeps free-form parsing (key=value pairs plus --json/--get/--raw
/// and --token-type) via [`crate::api::ApiCallArgs::parse`], so everything
/// after `call` is captured verbatim.
#[derive(Debug, Args)]
pub struct ApiCallCliArgs {
    /// Method followed by key=value parameters and api-call flags
    #[arg(
        value_name = "METHOD [key=value]...",
        trailing_var_arg = true,
        allow_hyphen_values = true,
        num_args = 0..
    )]
    pub rest: Vec<String>,
}

#[derive(Debug, Subcommand)]
pub enum AuthCommand {
    /// Authenticate with Slack via OAuth
    Login(LoginCliArgs),
    /// Show profile status
    Status {
        /// Profile name (defaults to 'default')
        profile_name: Option<String>,
    },
    /// List all profiles
    List,
    /// Rename a profile
    Rename {
        /// Current profile name
        old_name: String,
        /// New profile name
        new_name: String,
    },
    /// Remove authentication for a profile
    Logout {
        /// Profile name (defaults to 'default')
        profile_name: Option<String>,
    },
    /// Export profiles to encrypted file
    Export(ExportCliArgs),
    /// Import profiles from encrypted file
    Import(ImportCliArgs),
    /// Move legacy plaintext tokens.json into the OS keyring, then delete it
    Migrate {
        /// Path to the legacy tokens.json (defaults to the pre-keyring location)
        #[arg(long, value_name = "file")]
        path: Option<String>,
    },
}

#[derive(Debug, Args)]
pub struct LoginCliArgs {
    /// Profile name to store credentials under (defaults to 'default')
    pub profile_name: Option<String>,

    /// OAuth client ID
    #[arg(long, value_name = "id")]
    pub client_id: Option<String>,

    /// Bot scopes (comma-separated or 'all')
    #[arg(long, value_name = "scopes")]
    pub bot_scopes: Option<String>,

    /// User scopes (comma-separated or 'all')
    #[arg(long, value_name = "scopes")]
    pub user_scopes: Option<String>,

    /// Use cloudflared tunnel for the OAuth redirect URI
    /// (path optional, defaults to 'cloudflared' in PATH)
    #[arg(
        long,
        value_name = "path",
        num_args = 0..=1,
        default_missing_value = "cloudflared"
    )]
    pub cloudflared: Option<String>,
}

#[derive(Debug, Args)]
pub struct ExportCliArgs {
    /// Export all profiles
    #[arg(long)]
    pub all: bool,

    /// Output file path (required)
    #[arg(long, value_name = "file")]
    pub out: Option<String>,

    /// Environment variable containing passphrase
    #[arg(long, value_name = "var")]
    pub passphrase_env: Option<String>,

    /// Prompt for passphrase
    #[arg(long)]
    pub passphrase_prompt: bool,

    /// Confirm dangerous operation (required)
    #[arg(long)]
    pub yes: bool,

    /// Language code (en/ja)
    #[arg(long, value_name = "code")]
    pub lang: Option<String>,
}

#[derive(Debug, Args)]
pub struct ImportCliArgs {
    /// Input file path (required)
    #[arg(long = "in", value_name = "file")]
    pub input: Option<String>,

    /// Environment variable containing passphrase
    #[arg(long, value_name = "var")]
    pub passphrase_env: Option<String>,

    /// Prompt for passphrase
    #[arg(long)]
    pub passphrase_prompt: bool,

    /// Automatically accept conflicts
    #[arg(long)]
    pub yes: bool,

    /// Overwrite existing profiles
    #[arg(long)]
    pub force: bool,

    /// Preview changes without writing
    #[arg(long)]
    pub dry_run: bool,

    /// Output import result as JSON
    #[arg(long)]
    pub json: bool,

    /// Language code (en/ja)
    #[arg(long, value_name = "code")]
    pub lang: Option<String>,
}

#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    /// OAuth client configuration
    Oauth {
        #[command(subcommand)]
        command: ConfigOauthCommand,
    },
    /// Set default token type (bot/user) for a profile
    Set {
        /// Profile name
        profile_name: String,
        /// Default token type (bot or user)
        #[arg(long, value_name = "type", value_parser = token_type_value)]
        token_type: Option<TokenType>,
    },
}

#[derive(Debug, Subcommand)]
pub enum ConfigOauthCommand {
    /// Set OAuth configuration for a profile
    ///
    /// Client secret sources (priority order): --client-secret-env <VAR>,
    /// SLACKRS_CLIENT_SECRET env var, --client-secret-file <PATH>, or an
    /// interactive prompt when stdin is a TTY.
    Set(ConfigOauthSetArgs),
    /// Show OAuth configuration for a profile
    Show {
        /// Profile name
        profile_name: String,
    },
    /// Delete OAuth configuration for a profile
    Delete {
        /// Profile name
        profile_name: String,
    },
}

#[derive(Debug, Args)]
pub struct ConfigOauthSetArgs {
    /// Profile name
    pub profile_name: String,

    /// OAuth client ID
    #[arg(long, value_name = "id")]
    pub client_id: Option<String>,

    /// OAuth redirect URI
    #[arg(long, value_name = "uri")]
    pub redirect_uri: Option<String>,

    /// Comma-separated list of scopes or 'all'
    #[arg(long, value_name = "scopes")]
    pub scopes: Option<String>,

    /// Read client secret from environment variable
    #[arg(long, value_name = "VAR")]
    pub client_secret_env: Option<String>,

    /// Read client secret from file
    #[arg(long, value_name = "PATH")]
    pub client_secret_file: Option<String>,

    /// Removed for security — use --client-secret-env or --client-secret-file
    #[arg(long, hide = true, value_name = "SECRET")]
    pub client_secret: Option<String>,
}

#[derive(Debug, Args)]
pub struct SearchArgs {
    /// Search query
    pub query: String,

    /// Number of results
    #[arg(long, value_name = "N")]
    pub count: Option<u32>,

    /// Page number
    #[arg(long, value_name = "N")]
    pub page: Option<u32>,

    /// Sort type (score or timestamp)
    #[arg(long, value_name = "TYPE")]
    pub sort: Option<String>,

    /// Sort direction (asc or desc)
    #[arg(long = "sort_dir", value_name = "DIR")]
    pub sort_dir: Option<String>,

    /// Fetch all pages (starting from --page), merging matches
    #[arg(long)]
    pub all: bool,

    /// Safety cap on pages fetched with --all (429s are retried automatically)
    #[arg(long, value_name = "N", default_value_t = 10)]
    pub max_pages: u32,

    /// Token type to use (bot or user)
    #[arg(long, value_name = "TYPE", value_parser = token_type_value)]
    pub token_type: Option<TokenType>,

    /// Output raw Slack API response (without envelope)
    #[arg(long)]
    pub raw: bool,
}

#[derive(Debug, Subcommand)]
pub enum ConvCommand {
    /// List conversations with optional filtering and sorting
    List(ConvListArgs),
    /// Interactively select a conversation and output its channel ID
    Select(ConvSelectArgs),
    /// Search conversations by name pattern (applies name:<pattern> filter)
    Search(ConvSearchArgs),
    /// Get conversation history
    History(ConvHistoryArgs),
    /// Open (or resume) a DM/group DM with one or more users
    ///
    /// Wraps conversations.open. Idempotent and content-free (no message is
    /// posted, nobody is notified), so it is not gated by SLACKCLI_ALLOW_WRITE.
    Open(ConvOpenArgs),
}

#[derive(Debug, Args)]
pub struct ConvListArgs {
    /// Conversation types (comma-separated; mutually exclusive with --include-private/--all)
    #[arg(long, value_name = "TYPE")]
    pub types: Option<String>,

    /// Include private channels (same as default now)
    #[arg(long)]
    pub include_private: bool,

    /// Include all conversation types (public_channel,private_channel,im,mpim)
    #[arg(long)]
    pub all: bool,

    /// Maximum number of conversations
    #[arg(long, value_name = "N")]
    pub limit: Option<u32>,

    /// Filter (key:value; supports name:<glob>, is_member:BOOL, is_private:BOOL; repeatable)
    #[arg(long, value_name = "KEY:VALUE", action = ArgAction::Append)]
    pub filter: Vec<String>,

    /// Output format (json, jsonl, table, tsv)
    #[arg(long, value_name = "FORMAT")]
    pub format: Option<String>,

    /// Sort key (name, created, num_members)
    #[arg(long, value_name = "KEY")]
    pub sort: Option<String>,

    /// Sort direction (asc or desc)
    #[arg(long, value_name = "DIR")]
    pub sort_dir: Option<String>,

    /// Token type to use (bot or user)
    #[arg(long, value_name = "TYPE", value_parser = token_type_value)]
    pub token_type: Option<TokenType>,

    /// Output raw Slack API response (without envelope; only with --format json)
    #[arg(long)]
    pub raw: bool,
}

#[derive(Debug, Args)]
pub struct ConvSelectArgs {
    /// Conversation types (comma-separated)
    #[arg(long, value_name = "TYPE")]
    pub types: Option<String>,

    /// Maximum number of conversations
    #[arg(long, value_name = "N")]
    pub limit: Option<u32>,

    /// Filter (key:value format, repeatable)
    #[arg(long, value_name = "KEY:VALUE", action = ArgAction::Append)]
    pub filter: Vec<String>,

    /// Token type to use (bot or user)
    #[arg(long, value_name = "TYPE", value_parser = token_type_value)]
    pub token_type: Option<TokenType>,
}

#[derive(Debug, Args)]
pub struct ConvSearchArgs {
    /// Name pattern (supports * and ? wildcards)
    pub pattern: String,

    /// Interactively select from results and output channel ID only
    #[arg(long)]
    pub select: bool,

    /// Conversation types (comma-separated)
    #[arg(long, value_name = "TYPE")]
    pub types: Option<String>,

    /// Maximum number of conversations
    #[arg(long, value_name = "N")]
    pub limit: Option<u32>,

    /// Filter (key:value format, repeatable)
    #[arg(long, value_name = "KEY:VALUE", action = ArgAction::Append)]
    pub filter: Vec<String>,

    /// Output format (json, jsonl, table, tsv)
    #[arg(long, value_name = "FORMAT")]
    pub format: Option<String>,

    /// Sort key (name, created, num_members)
    #[arg(long, value_name = "KEY")]
    pub sort: Option<String>,

    /// Sort direction (asc or desc)
    #[arg(long, value_name = "DIR")]
    pub sort_dir: Option<String>,

    /// Token type to use (bot or user)
    #[arg(long, value_name = "TYPE", value_parser = token_type_value)]
    pub token_type: Option<TokenType>,

    /// Output raw Slack API response (without envelope; only with --format json)
    #[arg(long)]
    pub raw: bool,
}

#[derive(Debug, Args)]
pub struct ConvHistoryArgs {
    /// Channel ID (required unless --interactive)
    pub channel: Option<String>,

    /// Select channel interactively before fetching history
    #[arg(long)]
    pub interactive: bool,

    /// Conversation types for interactive selection (comma-separated)
    #[arg(long, value_name = "TYPE")]
    pub types: Option<String>,

    /// Filter for interactive selection (key:value format, repeatable)
    #[arg(long, value_name = "KEY:VALUE", action = ArgAction::Append)]
    pub filter: Vec<String>,

    /// Maximum number of messages
    #[arg(long, value_name = "N")]
    pub limit: Option<u32>,

    /// Only messages after this timestamp
    #[arg(long, value_name = "TS")]
    pub oldest: Option<String>,

    /// Only messages before this timestamp
    #[arg(long, value_name = "TS")]
    pub latest: Option<String>,

    /// Start fetching from this pagination cursor
    #[arg(long, value_name = "CURSOR")]
    pub cursor: Option<String>,

    /// Follow next_cursor until exhausted (bounded by --max-pages)
    #[arg(long)]
    pub all: bool,

    /// Safety cap on pages fetched with --all (429s are retried automatically)
    #[arg(long, value_name = "N", default_value_t = 10)]
    pub max_pages: u32,

    /// Token type to use (bot or user)
    #[arg(long, value_name = "TYPE", value_parser = token_type_value)]
    pub token_type: Option<TokenType>,

    /// Output raw Slack API response (without envelope)
    #[arg(long)]
    pub raw: bool,
}

#[derive(Debug, Args)]
pub struct ConvOpenArgs {
    /// One or more user IDs (2+ IDs open a group DM)
    #[arg(value_name = "USER_ID", required = true, num_args = 1..)]
    pub user_ids: Vec<String>,

    /// Token type to use (bot or user)
    #[arg(long, value_name = "TYPE", value_parser = token_type_value)]
    pub token_type: Option<TokenType>,

    /// Output raw Slack API response (without envelope)
    #[arg(long)]
    pub raw: bool,
}

#[derive(Debug, Subcommand)]
pub enum ThreadCommand {
    /// Get thread messages (conversation replies) for a specific thread
    ///
    /// Default JSON wraps the Slack response and adds response.resolved_users
    /// at thread scope. --raw returns the Slack-native conversations.replies
    /// shape without wrapper metadata. Fetches a single page by default; use
    /// --all to follow next_cursor (bounded by --max-pages) and --cursor to
    /// resume from a previous page.
    Get(ThreadGetArgs),
}

#[derive(Debug, Args)]
pub struct ThreadGetArgs {
    /// Channel ID containing the thread
    pub channel: String,

    /// Timestamp of the parent message (thread identifier)
    pub thread_ts: String,

    /// Number of messages per page (default: 100)
    #[arg(long, value_name = "N")]
    pub limit: Option<u32>,

    /// Include the parent message in results
    #[arg(long)]
    pub inclusive: bool,

    /// Start fetching from this pagination cursor
    #[arg(long, value_name = "CURSOR")]
    pub cursor: Option<String>,

    /// Follow next_cursor until exhausted (bounded by --max-pages)
    #[arg(long)]
    pub all: bool,

    /// Safety cap on pages fetched with --all (429s are retried automatically)
    #[arg(long, value_name = "N", default_value_t = 10)]
    pub max_pages: u32,

    /// Token type to use (bot or user)
    #[arg(long, value_name = "TYPE", value_parser = token_type_value)]
    pub token_type: Option<TokenType>,

    /// Output raw Slack API response (without envelope)
    #[arg(long)]
    pub raw: bool,
}

#[derive(Debug, Subcommand)]
pub enum UsersCommand {
    /// Get user information
    Info(UsersInfoArgs),
    /// Look up a user by email address
    Lookup(UsersLookupArgs),
    /// Update user cache for mention resolution
    CacheUpdate(UsersCacheUpdateArgs),
    /// Resolve user mentions in text
    ResolveMentions(UsersResolveMentionsArgs),
}

#[derive(Debug, Args)]
pub struct UsersInfoArgs {
    /// User ID (e.g. U123456)
    pub user_id: String,

    /// Token type to use (bot or user)
    #[arg(long, value_name = "TYPE", value_parser = token_type_value)]
    pub token_type: Option<TokenType>,

    /// Output raw Slack API response (without envelope)
    #[arg(long)]
    pub raw: bool,
}

#[derive(Debug, Args)]
pub struct UsersLookupArgs {
    /// Email address to look up (users.lookupByEmail)
    #[arg(long, value_name = "EMAIL", required = true)]
    pub email: String,

    /// Token type to use (bot or user)
    #[arg(long, value_name = "TYPE", value_parser = token_type_value)]
    pub token_type: Option<TokenType>,

    /// Output raw Slack API response (without envelope)
    #[arg(long)]
    pub raw: bool,
}

#[derive(Debug, Args)]
pub struct UsersCacheUpdateArgs {
    /// Force cache update
    #[arg(long)]
    pub force: bool,

    /// Token type to use (bot or user)
    #[arg(long, value_name = "TYPE", value_parser = token_type_value)]
    pub token_type: Option<TokenType>,
}

#[derive(Debug, Args)]
pub struct UsersResolveMentionsArgs {
    /// Text containing <@U123> mentions
    pub text: String,

    /// Mention format (display_name, real_name, or username)
    #[arg(long, value_name = "FORMAT")]
    pub format: Option<String>,
}

#[derive(Debug, Subcommand)]
pub enum MsgCommand {
    /// Post a message (requires SLACKCLI_ALLOW_WRITE=true)
    Post(MsgPostArgs),
    /// Update a message (requires SLACKCLI_ALLOW_WRITE=true)
    Update(MsgUpdateArgs),
    /// Delete a message (requires SLACKCLI_ALLOW_WRITE=true)
    Delete(MsgDeleteArgs),
}

#[derive(Debug, Args)]
pub struct MsgPostArgs {
    /// Channel ID (omit when using --user; with --user, a single positional
    /// argument is treated as the message text)
    pub channel: Option<String>,

    /// Message text (fallback text when --blocks is provided; required
    /// unless --blocks is given)
    pub text: Option<String>,

    /// Post to a DM with this user ID instead of a channel
    /// (opens the DM via conversations.open first; mutually exclusive with
    /// the positional channel argument)
    #[arg(long, value_name = "USER_ID")]
    pub user: Option<String>,

    /// Block Kit blocks as a JSON array, or @<path> to read from a file
    #[arg(long, value_name = "JSON|@FILE")]
    pub blocks: Option<String>,

    /// Thread timestamp for reply
    #[arg(long, value_name = "TS")]
    pub thread_ts: Option<String>,

    /// Broadcast reply to channel (requires --thread-ts)
    #[arg(long)]
    pub reply_broadcast: bool,

    /// Skip confirmation prompt
    #[arg(long)]
    pub yes: bool,

    /// Idempotency key for preventing duplicate operations
    #[arg(long, value_name = "KEY")]
    pub idempotency_key: Option<String>,

    /// Token type to use (bot or user)
    #[arg(long, value_name = "TYPE", value_parser = token_type_value)]
    pub token_type: Option<TokenType>,

    /// Output raw Slack API response (without envelope)
    #[arg(long)]
    pub raw: bool,
}

#[derive(Debug, Args)]
pub struct MsgUpdateArgs {
    /// Channel ID
    pub channel: String,

    /// Message timestamp
    pub ts: String,

    /// New message text
    pub text: String,

    /// Skip confirmation prompt
    #[arg(long)]
    pub yes: bool,

    /// Idempotency key for preventing duplicate operations
    #[arg(long, value_name = "KEY")]
    pub idempotency_key: Option<String>,

    /// Token type to use (bot or user)
    #[arg(long, value_name = "TYPE", value_parser = token_type_value)]
    pub token_type: Option<TokenType>,

    /// Output raw Slack API response (without envelope)
    #[arg(long)]
    pub raw: bool,
}

#[derive(Debug, Args)]
pub struct MsgDeleteArgs {
    /// Channel ID
    pub channel: String,

    /// Message timestamp
    pub ts: String,

    /// Skip confirmation prompt
    #[arg(long)]
    pub yes: bool,

    /// Idempotency key for preventing duplicate operations
    #[arg(long, value_name = "KEY")]
    pub idempotency_key: Option<String>,

    /// Token type to use (bot or user)
    #[arg(long, value_name = "TYPE", value_parser = token_type_value)]
    pub token_type: Option<TokenType>,

    /// Output raw Slack API response (without envelope)
    #[arg(long)]
    pub raw: bool,
}

#[derive(Debug, Subcommand)]
pub enum ReactCommand {
    /// Add a reaction (requires SLACKCLI_ALLOW_WRITE=true)
    Add(ReactArgs),
    /// Remove a reaction (requires SLACKCLI_ALLOW_WRITE=true)
    Remove(ReactArgs),
}

#[derive(Debug, Args)]
pub struct ReactArgs {
    /// Channel ID
    pub channel: String,

    /// Message timestamp
    pub ts: String,

    /// Emoji name (without colons)
    pub emoji: String,

    /// Skip confirmation prompt
    #[arg(long)]
    pub yes: bool,

    /// Idempotency key for preventing duplicate operations
    #[arg(long, value_name = "KEY")]
    pub idempotency_key: Option<String>,

    /// Token type to use (bot or user)
    #[arg(long, value_name = "TYPE", value_parser = token_type_value)]
    pub token_type: Option<TokenType>,

    /// Output raw Slack API response (without envelope)
    #[arg(long)]
    pub raw: bool,
}

#[derive(Debug, Subcommand)]
pub enum FileCommand {
    /// Upload a file using the external upload method (requires SLACKCLI_ALLOW_WRITE=true)
    Upload(FileUploadArgs),
    /// Download a file from Slack (either <file_id> or --url is required)
    Download(FileDownloadArgs),
}

#[derive(Debug, Args)]
pub struct FileUploadArgs {
    /// Path of the file to upload
    pub path: String,

    /// Channel ID to share the file in
    #[arg(long, value_name = "ID")]
    pub channel: Option<String>,

    /// Channel IDs to share the file in (alias of --channel)
    #[arg(long, value_name = "IDs")]
    pub channels: Option<String>,

    /// File title
    #[arg(long, value_name = "TITLE")]
    pub title: Option<String>,

    /// Initial comment
    #[arg(long, value_name = "TEXT")]
    pub comment: Option<String>,

    /// Skip confirmation prompt
    #[arg(long)]
    pub yes: bool,

    /// Idempotency key for preventing duplicate operations
    #[arg(long, value_name = "KEY")]
    pub idempotency_key: Option<String>,

    /// Token type to use (bot or user)
    #[arg(long, value_name = "TYPE", value_parser = token_type_value)]
    pub token_type: Option<TokenType>,

    /// Output raw Slack API response (without envelope)
    #[arg(long)]
    pub raw: bool,
}

#[derive(Debug, Args)]
pub struct FileDownloadArgs {
    /// File ID (alternative to --url)
    pub file_id: Option<String>,

    /// Direct download URL (alternative to file_id)
    #[arg(long, value_name = "URL")]
    pub url: Option<String>,

    /// Output path (omit for current directory, '-' for stdout, directory for auto-naming)
    #[arg(long, value_name = "PATH")]
    pub out: Option<String>,

    /// Token type to use (bot or user)
    #[arg(long, value_name = "TYPE", value_parser = token_type_value)]
    pub token_type: Option<TokenType>,

    /// Output raw response (without envelope)
    #[arg(long)]
    pub raw: bool,
}

#[derive(Debug, Args)]
pub struct DoctorArgs {
    /// Output in JSON format
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct InstallSkillsArgs {
    /// Source to install from: 'self' (embedded) or 'local:<path>'
    pub source: Option<String>,

    /// Install to ~/.agents instead of ./.agents
    #[arg(long)]
    pub global: bool,

    /// Output installation result as JSON
    #[arg(long)]
    pub json: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;
    use clap::Parser;

    fn parse(argv: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(argv)
    }

    #[test]
    fn test_cli_asserts() {
        Cli::command().debug_assert();
    }

    #[test]
    fn test_global_flags_any_position() {
        let cli = parse(&["slack", "--profile", "work", "conv", "list"]).unwrap();
        assert_eq!(cli.profile.as_deref(), Some("work"));

        let cli = parse(&["slack", "conv", "list", "--profile", "work"]).unwrap();
        assert_eq!(cli.profile.as_deref(), Some("work"));

        let cli = parse(&["slack", "conv", "list", "--profile=work"]).unwrap();
        assert_eq!(cli.profile.as_deref(), Some("work"));

        let cli = parse(&["slack", "--non-interactive", "search", "q"]).unwrap();
        assert!(cli.non_interactive);
    }

    #[test]
    fn test_api_call_trailing_args() {
        let cli = parse(&[
            "slack",
            "api",
            "call",
            "chat.postMessage",
            "channel=C123",
            "text=Hello",
            "--json",
            "--get",
            "--raw",
            "--token-type=user",
        ])
        .unwrap();
        match cli.command {
            Command::Api {
                command: ApiCommand::Call(args),
            } => {
                assert_eq!(args.rest[0], "chat.postMessage");
                assert!(args.rest.contains(&"--json".to_string()));
                assert!(args.rest.contains(&"--token-type=user".to_string()));
            }
            _ => panic!("expected api call"),
        }
    }

    #[test]
    fn test_login_cloudflared_optional_value() {
        let cli = parse(&["slack", "auth", "login", "--cloudflared"]).unwrap();
        match cli.command {
            Command::Auth {
                command: AuthCommand::Login(args),
            } => assert_eq!(args.cloudflared.as_deref(), Some("cloudflared")),
            _ => panic!("expected auth login"),
        }

        let cli = parse(&[
            "slack",
            "auth",
            "login",
            "--cloudflared",
            "/usr/bin/cloudflared",
        ])
        .unwrap();
        match cli.command {
            Command::Auth {
                command: AuthCommand::Login(args),
            } => assert_eq!(args.cloudflared.as_deref(), Some("/usr/bin/cloudflared")),
            _ => panic!("expected auth login"),
        }
    }

    #[test]
    fn test_login_rejects_unknown_option() {
        assert!(parse(&["slack", "auth", "login", "--ngrok"]).is_err());
        assert!(parse(&["slack", "auth", "login", "p1", "p2"]).is_err());
    }

    #[test]
    fn test_conv_list_filters_repeatable_both_formats() {
        let cli = parse(&[
            "slack",
            "conv",
            "list",
            "--filter=is_private:true",
            "--filter",
            "is_member:true",
        ])
        .unwrap();
        match cli.command {
            Command::Conv {
                command: ConvCommand::List(args),
            } => assert_eq!(args.filter, vec!["is_private:true", "is_member:true"]),
            _ => panic!("expected conv list"),
        }
    }

    #[test]
    fn test_msg_post_flags() {
        let cli = parse(&[
            "slack",
            "msg",
            "post",
            "C123",
            "hello",
            "--thread-ts=123.456",
            "--reply-broadcast",
            "--idempotency-key",
            "k1",
        ])
        .unwrap();
        match cli.command {
            Command::Msg {
                command: MsgCommand::Post(args),
            } => {
                assert_eq!(args.channel.as_deref(), Some("C123"));
                assert_eq!(args.text.as_deref(), Some("hello"));
                assert_eq!(args.thread_ts.as_deref(), Some("123.456"));
                assert!(args.reply_broadcast);
                assert_eq!(args.idempotency_key.as_deref(), Some("k1"));
            }
            _ => panic!("expected msg post"),
        }
    }

    #[test]
    fn test_token_type_value_parsing() {
        let cli = parse(&["slack", "thread", "get", "C1", "1.2", "--token-type", "bot"]).unwrap();
        match cli.command {
            Command::Thread {
                command: ThreadCommand::Get(args),
            } => assert_eq!(args.token_type, Some(TokenType::Bot)),
            _ => panic!("expected thread get"),
        }

        assert!(parse(&["slack", "thread", "get", "C1", "1.2", "--token-type", "bad"]).is_err());
    }

    #[test]
    fn test_search_sort_dir_underscore_name() {
        let cli = parse(&["slack", "search", "q", "--sort_dir", "asc"]).unwrap();
        match cli.command {
            Command::Search(args) => assert_eq!(args.sort_dir.as_deref(), Some("asc")),
            _ => panic!("expected search"),
        }
    }

    #[test]
    fn test_required_positionals_missing() {
        assert!(parse(&["slack", "thread", "get", "C1"]).is_err());
        // msg post positionals are validated at runtime (channel/text are
        // optional at parse time to support --user and --blocks)
        assert!(parse(&["slack", "react", "add", "C1", "1.2"]).is_err());
        assert!(parse(&["slack", "auth", "rename", "only-one"]).is_err());
        assert!(parse(&["slack", "search"]).is_err());
        assert!(parse(&["slack", "conv", "search"]).is_err());
        assert!(parse(&["slack", "conv", "open"]).is_err());
        assert!(parse(&["slack", "users", "lookup"]).is_err());
    }

    #[test]
    fn test_msg_post_user_and_blocks_flags() {
        let cli = parse(&[
            "slack", "msg", "post", "--user", "U123", "hello", "--blocks", "[]",
        ])
        .unwrap();
        match cli.command {
            Command::Msg {
                command: MsgCommand::Post(args),
            } => {
                assert_eq!(args.user.as_deref(), Some("U123"));
                // Single positional lands in the channel slot; runtime
                // resolution treats it as text when --user is present.
                assert_eq!(args.channel.as_deref(), Some("hello"));
                assert!(args.text.is_none());
                assert_eq!(args.blocks.as_deref(), Some("[]"));
            }
            _ => panic!("expected msg post"),
        }

        // --blocks alone (no text) parses; text requirement is runtime-checked
        assert!(parse(&["slack", "msg", "post", "C1", "--blocks", "[]"]).is_ok());
    }

    #[test]
    fn test_conv_open_args() {
        let cli = parse(&["slack", "conv", "open", "U1", "U2", "--token-type", "user"]).unwrap();
        match cli.command {
            Command::Conv {
                command: ConvCommand::Open(args),
            } => {
                assert_eq!(args.user_ids, vec!["U1", "U2"]);
                assert_eq!(args.token_type, Some(TokenType::User));
            }
            _ => panic!("expected conv open"),
        }
    }

    #[test]
    fn test_users_lookup_args() {
        let cli = parse(&["slack", "users", "lookup", "--email", "a@b.co"]).unwrap();
        match cli.command {
            Command::Users {
                command: UsersCommand::Lookup(args),
            } => assert_eq!(args.email, "a@b.co"),
            _ => panic!("expected users lookup"),
        }
    }

    #[test]
    fn test_pagination_flags() {
        let cli = parse(&[
            "slack",
            "conv",
            "history",
            "C1",
            "--cursor",
            "cur1",
            "--all",
            "--max-pages",
            "3",
        ])
        .unwrap();
        match cli.command {
            Command::Conv {
                command: ConvCommand::History(args),
            } => {
                assert_eq!(args.cursor.as_deref(), Some("cur1"));
                assert!(args.all);
                assert_eq!(args.max_pages, 3);
            }
            _ => panic!("expected conv history"),
        }

        let cli = parse(&["slack", "thread", "get", "C1", "1.2", "--all"]).unwrap();
        match cli.command {
            Command::Thread {
                command: ThreadCommand::Get(args),
            } => {
                assert!(args.all);
                assert_eq!(args.max_pages, 10);
                assert!(args.cursor.is_none());
            }
            _ => panic!("expected thread get"),
        }

        let cli = parse(&["slack", "search", "q", "--all", "--max-pages", "2"]).unwrap();
        match cli.command {
            Command::Search(args) => {
                assert!(args.all);
                assert_eq!(args.max_pages, 2);
            }
            _ => panic!("expected search"),
        }
    }

    #[test]
    fn test_auth_migrate_path() {
        let cli = parse(&["slack", "auth", "migrate", "--path", "/tmp/tokens.json"]).unwrap();
        match cli.command {
            Command::Auth {
                command: AuthCommand::Migrate { path },
            } => assert_eq!(path.as_deref(), Some("/tmp/tokens.json")),
            _ => panic!("expected auth migrate"),
        }
    }

    #[test]
    fn test_completions_command() {
        assert!(parse(&["slack", "completions", "bash"]).is_ok());
        assert!(parse(&["slack", "completions", "zsh"]).is_ok());
        assert!(parse(&["slack", "completions", "fish"]).is_ok());
        assert!(parse(&["slack", "completions", "not-a-shell"]).is_err());
    }
}
