//! Unified output envelope for all commands
//!
//! Provides a consistent output structure with response and metadata

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Unified command response with envelope
#[derive(Debug, Serialize, Deserialize)]
pub struct CommandResponse {
    /// Schema version for introspection
    #[serde(rename = "schemaVersion")]
    pub schema_version: u32,

    /// Response type identifier for introspection
    #[serde(rename = "type")]
    pub response_type: String,

    /// Indicates if the operation was successful
    pub ok: bool,

    /// Original API response
    pub response: Value,

    /// Execution metadata
    pub meta: CommandMeta,
}

/// Command execution metadata
#[derive(Debug, Serialize, Deserialize)]
pub struct CommandMeta {
    pub profile_name: Option<String>,
    pub team_id: String,
    pub user_id: String,
    pub method: String,
    pub command: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub idempotency_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pagination: Option<PaginationMeta>,
}

/// Pagination metadata for commands that page through Slack results.
///
/// `next_cursor` (cursor-based APIs) or `next_page` (page-based APIs, e.g.
/// search.messages) is present only when more results remain — i.e. the
/// output was truncated by the page limit (`--max-pages`) or because `--all`
/// was not requested.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaginationMeta {
    /// Number of pages fetched from the Slack API
    pub pages_fetched: u32,
    /// Cursor to resume from when results were truncated (cursor-based APIs)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    /// Page number to resume from when results were truncated (page-based APIs)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_page: Option<u32>,
}

impl CommandResponse {
    /// Create a new command response with metadata
    pub fn new(
        response: Value,
        profile_name: Option<String>,
        team_id: String,
        user_id: String,
        method: String,
        command: String,
    ) -> Self {
        // Extract 'ok' from Slack API response if present
        let ok = response
            .as_object()
            .and_then(|obj| obj.get("ok"))
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        // Generate type from method (e.g., "conversations.list" -> "conversations.list")
        let response_type = method.clone();

        Self {
            schema_version: 1,
            response_type,
            ok,
            response,
            meta: CommandMeta {
                profile_name,
                team_id,
                user_id,
                method,
                command,
                token_type: None,
                idempotency_key: None,
                idempotency_status: None,
                pagination: None,
            },
        }
    }

    /// Create a new command response with metadata including token type
    pub fn with_token_type(
        response: Value,
        profile_name: Option<String>,
        team_id: String,
        user_id: String,
        method: String,
        command: String,
        token_type: Option<String>,
    ) -> Self {
        // Extract 'ok' from Slack API response if present
        let ok = response
            .as_object()
            .and_then(|obj| obj.get("ok"))
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        // Generate type from method (e.g., "conversations.list" -> "conversations.list")
        let response_type = method.clone();

        Self {
            schema_version: 1,
            response_type,
            ok,
            response,
            meta: CommandMeta {
                profile_name,
                team_id,
                user_id,
                method,
                command,
                token_type,
                idempotency_key: None,
                idempotency_status: None,
                pagination: None,
            },
        }
    }

    /// Set idempotency metadata
    pub fn with_idempotency(mut self, key: String, status: String) -> Self {
        self.meta.idempotency_key = Some(key);
        self.meta.idempotency_status = Some(status);
        self
    }

    /// Set pagination metadata
    pub fn with_pagination(mut self, pagination: PaginationMeta) -> Self {
        self.meta.pagination = Some(pagination);
        self
    }
}
