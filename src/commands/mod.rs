//! Command implementations for Slack CLI wrapper commands
//!
//! Provides high-level commands that wrap the generic API client:
//! - search: Search messages
//! - conv: Conversation operations (list, history)
//! - thread: Thread operations (get replies)
//! - users: User operations (info)
//! - users_cache: User cache and mention resolution
//! - msg: Message operations (post, update, delete)
//! - react: Reaction operations (add, remove)
//! - file: File operations (upload using external upload method)
//! - config: Configuration management (OAuth settings)
//! - doctor: Diagnostics and environment troubleshooting

pub mod config;
pub mod conv;
pub mod doctor;
pub mod file;
pub mod guards;
pub mod msg;
pub mod paging;
pub mod react;
pub mod search;
pub mod thread;
pub mod users;
pub mod users_cache;

pub use config::{oauth_delete, oauth_set, oauth_show, set_default_token_type, OAuthSetParams};
pub use conv::{
    apply_filters, conv_history, conv_history_paged, conv_list, conv_open, extract_conversations,
    extract_opened_channel_id, format_response, sort_conversations, ConversationFilter,
    ConversationItem, ConversationSelector, OutputFormat, SortDirection, SortKey, StdinSelector,
};
pub use doctor::doctor;
pub use file::{file_download, file_upload};
pub use msg::{msg_delete, msg_post, msg_update, MsgPostParams};
pub use paging::{PageOptions, DEFAULT_MAX_PAGES};
pub use react::{react_add, react_remove};
pub use search::{search, search_paged, SearchRequest};
pub use thread::{thread_get, thread_get_paged};
pub use users::{users_info, users_lookup_by_email};
pub use users_cache::{resolve_mentions, update_cache, MentionFormat, UsersCacheFile};
