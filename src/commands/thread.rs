//! Thread operations - retrieve thread messages

use crate::api::{ApiClient, ApiError, ApiMethod, ApiResponse, PaginationMeta};
use crate::commands::paging::{paginate_messages, PageOptions};
use serde_json::json;
use std::collections::HashMap;

/// Page cap used by the legacy full-pagination [`thread_get`] wrapper
/// (guards against infinite loops, matching the pre-`--all` behavior).
const LEGACY_MAX_PAGES: u32 = 1000;

/// Get thread messages (conversation replies), following all pages
///
/// Legacy library entry point that aggregates every page (up to an internal
/// safety cap). The CLI `thread get` command uses [`thread_get_paged`] with
/// explicit `--cursor` / `--all` / `--max-pages` control instead.
pub async fn thread_get(
    client: &ApiClient,
    channel: String,
    thread_ts: String,
    limit: Option<u32>,
    inclusive: Option<bool>,
) -> Result<ApiResponse, ApiError> {
    let paging = PageOptions {
        cursor: None,
        all: true,
        max_pages: LEGACY_MAX_PAGES,
    };
    let (response, _) =
        thread_get_paged(client, channel, thread_ts, limit, inclusive, &paging).await?;
    Ok(response)
}

/// Get thread messages (conversation replies) with explicit pagination control
///
/// # Arguments
/// * `client` - API client
/// * `channel` - Channel ID containing the thread
/// * `thread_ts` - Timestamp of the parent message (thread identifier)
/// * `limit` - Optional number of messages per page (default: 100)
/// * `inclusive` - Optional flag to include the parent message (default: false)
/// * `paging` - Cursor/`--all`/`--max-pages` pagination options
///
/// # Pagination
/// Fetches a single page by default (starting from `paging.cursor` when
/// set). With `paging.all`, follows `next_cursor` until exhausted or the
/// `paging.max_pages` safety cap is reached, aggregating the `messages`
/// array. 429 responses are retried by the client. The returned
/// [`PaginationMeta`] reports pages fetched and the resume cursor when
/// results were truncated.
pub async fn thread_get_paged(
    client: &ApiClient,
    channel: String,
    thread_ts: String,
    limit: Option<u32>,
    inclusive: Option<bool>,
    paging: &PageOptions,
) -> Result<(ApiResponse, PaginationMeta), ApiError> {
    let mut params = HashMap::new();
    params.insert("channel".to_string(), json!(channel));
    params.insert("ts".to_string(), json!(thread_ts));

    // Use provided limit or default to 100
    let page_limit = limit.unwrap_or(100);
    params.insert("limit".to_string(), json!(page_limit));

    if let Some(incl) = inclusive {
        params.insert("inclusive".to_string(), json!(incl));
    }

    paginate_messages(client, ApiMethod::ConversationsReplies, params, paging).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_thread_get_basic() {
        let client = ApiClient::with_token("test_token".to_string());
        let result = thread_get(
            &client,
            "C123456".to_string(),
            "1234567890.123456".to_string(),
            None,
            None,
        )
        .await;
        // Result will fail because there's no mock server, but that's expected
        assert!(result.is_err());
    }
}
