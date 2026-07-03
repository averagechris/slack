//! Shared pagination options for wrapper commands that page through
//! cursor-based Slack APIs (`conversations.history`, `conversations.replies`).
//!
//! Rate-limited (429) responses inside a pagination loop are retried
//! transparently by [`crate::api::ApiClient::call_method`].

use crate::api::{ApiClient, ApiError, ApiMethod, ApiResponse, PaginationMeta};
use serde_json::{json, Value};
use std::collections::HashMap;

/// Default safety cap on the number of pages fetched with `--all`.
pub const DEFAULT_MAX_PAGES: u32 = 10;

/// Options controlling cursor-based pagination for a wrapper command.
#[derive(Debug, Clone)]
pub struct PageOptions {
    /// Cursor to start fetching from (Slack `cursor` parameter)
    pub cursor: Option<String>,
    /// Follow `next_cursor` until exhausted (bounded by `max_pages`)
    pub all: bool,
    /// Safety cap on pages fetched when `all` is set
    pub max_pages: u32,
}

impl Default for PageOptions {
    fn default() -> Self {
        Self {
            cursor: None,
            all: false,
            max_pages: DEFAULT_MAX_PAGES,
        }
    }
}

impl PageOptions {
    /// Fetch a single page, optionally starting from a cursor.
    pub fn single_page(cursor: Option<String>) -> Self {
        Self {
            cursor,
            all: false,
            ..Self::default()
        }
    }
}

/// Fetch one or more pages of a cursor-paginated method, aggregating the
/// `messages` array across pages.
///
/// The first page's response fields are preserved; `messages` is replaced
/// with the aggregated array and `response_metadata.next_cursor` reflects
/// the resume cursor ("" when exhausted). Follows `next_cursor` only when
/// `paging.all` is set, capped at `paging.max_pages` pages. 429 responses
/// are retried by the underlying client.
pub(crate) async fn paginate_messages(
    client: &ApiClient,
    method: ApiMethod,
    base_params: HashMap<String, Value>,
    paging: &PageOptions,
) -> Result<(ApiResponse, PaginationMeta), ApiError> {
    let mut all_messages: Vec<Value> = Vec::new();
    let mut cursor = paging.cursor.clone();
    let mut pages_fetched: u32 = 0;
    let mut first_response: Option<ApiResponse> = None;
    let mut seen_cursors: Vec<String> = Vec::new();

    loop {
        let mut params = base_params.clone();
        if let Some(ref cursor_val) = cursor {
            params.insert("cursor".to_string(), json!(cursor_val));
        }

        let response = client.call_method(method.clone(), params).await?;
        pages_fetched += 1;

        if let Some(messages) = response.data.get("messages").and_then(|m| m.as_array()) {
            all_messages.extend(messages.clone());
        }

        cursor = response
            .data
            .get("response_metadata")
            .and_then(|meta| meta.get("next_cursor"))
            .and_then(|c| c.as_str())
            .filter(|c| !c.is_empty())
            .map(|c| c.to_string());

        if first_response.is_none() {
            first_response = Some(response);
        }

        match cursor {
            None => break,
            Some(ref next) => {
                // Stop on single-page mode, page cap, or a repeated cursor
                // (defensive: avoids infinite loops on a misbehaving server).
                if !paging.all
                    || pages_fetched >= paging.max_pages
                    || seen_cursors.iter().any(|c| c == next)
                {
                    break;
                }
                seen_cursors.push(next.clone());
            }
        }
    }

    let mut response = first_response.expect("at least one page fetched");
    response
        .data
        .insert("messages".to_string(), json!(all_messages));
    response.data.insert(
        "response_metadata".to_string(),
        json!({ "next_cursor": cursor.clone().unwrap_or_default() }),
    );
    if response.data.contains_key("has_more") {
        response
            .data
            .insert("has_more".to_string(), json!(cursor.is_some()));
    }

    let pagination = PaginationMeta {
        pages_fetched,
        next_cursor: cursor,
        next_page: None,
    };

    Ok((response, pagination))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_page_options_default() {
        let opts = PageOptions::default();
        assert!(opts.cursor.is_none());
        assert!(!opts.all);
        assert_eq!(opts.max_pages, DEFAULT_MAX_PAGES);
    }

    #[test]
    fn test_page_options_for_single_page() {
        let opts = PageOptions::single_page(Some("cur".to_string()));
        assert_eq!(opts.cursor.as_deref(), Some("cur"));
        assert!(!opts.all);
    }
}
