//! Search command implementation

use crate::api::{ApiClient, ApiError, ApiMethod, ApiResponse, PaginationMeta};
use serde_json::json;
use std::collections::HashMap;

/// Search messages in Slack
///
/// # Arguments
/// * `client` - API client
/// * `query` - Search query string
/// * `count` - Optional number of results to return (default: 20)
/// * `page` - Optional page number (default: 1)
/// * `sort` - Optional sort order: "score" or "timestamp"
/// * `sort_dir` - Optional sort direction: "asc" or "desc"
///
/// # Returns
/// * `Ok(ApiResponse)` with search results
/// * `Err(ApiError)` if the operation fails
pub async fn search(
    client: &ApiClient,
    query: String,
    count: Option<u32>,
    page: Option<u32>,
    sort: Option<String>,
    sort_dir: Option<String>,
) -> Result<ApiResponse, ApiError> {
    let mut params = HashMap::new();
    params.insert("query".to_string(), json!(query));

    if let Some(count) = count {
        params.insert("count".to_string(), json!(count));
    }

    if let Some(page) = page {
        params.insert("page".to_string(), json!(page));
    }

    if let Some(sort) = sort {
        params.insert("sort".to_string(), json!(sort));
    }

    if let Some(sort_dir) = sort_dir {
        params.insert("sort_dir".to_string(), json!(sort_dir));
    }

    client.call_method(ApiMethod::SearchMessages, params).await
}

/// Search request with page-based pagination control
///
/// `search.messages` uses page-based pagination (`page` / `paging.pages`),
/// not cursors. With `all`, pages are fetched starting from `page`
/// (default: page 1) until the API reports no more pages or `max_pages` is
/// reached, and `messages.matches` are merged across pages.
#[derive(Debug, Clone)]
pub struct SearchRequest {
    pub query: String,
    pub count: Option<u32>,
    pub page: Option<u32>,
    pub sort: Option<String>,
    pub sort_dir: Option<String>,
    /// Follow pages until exhausted (bounded by `max_pages`)
    pub all: bool,
    /// Safety cap on pages fetched when `all` is set
    pub max_pages: u32,
}

/// Extract `messages.paging.pages` (total pages) from a search response.
fn total_pages(response: &ApiResponse) -> Option<u32> {
    response
        .data
        .get("messages")?
        .get("paging")?
        .get("pages")?
        .as_u64()
        .map(|p| p as u32)
}

/// Extract `messages.matches` from a search response.
fn matches_of(response: &ApiResponse) -> Vec<serde_json::Value> {
    response
        .data
        .get("messages")
        .and_then(|m| m.get("matches"))
        .and_then(|m| m.as_array())
        .cloned()
        .unwrap_or_default()
}

/// Search messages with pagination metadata (used by the `search` command)
///
/// Fetches a single page unless `req.all` is set, in which case matches are
/// merged across pages up to `req.max_pages`. 429 responses are retried by
/// the client. The returned [`PaginationMeta`] reports pages fetched and
/// `next_page` when more results remain.
pub async fn search_paged(
    client: &ApiClient,
    req: &SearchRequest,
) -> Result<(ApiResponse, PaginationMeta), ApiError> {
    let start_page = req.page.unwrap_or(1).max(1);
    let mut current_page = start_page;
    let mut pages_fetched: u32 = 0;
    let mut all_matches: Vec<serde_json::Value> = Vec::new();
    let mut first_response: Option<ApiResponse> = None;
    let mut next_page: Option<u32> = None;

    loop {
        let response = search(
            client,
            req.query.clone(),
            req.count,
            Some(current_page),
            req.sort.clone(),
            req.sort_dir.clone(),
        )
        .await?;
        pages_fetched += 1;

        all_matches.extend(matches_of(&response));
        let pages = total_pages(&response);
        if first_response.is_none() {
            first_response = Some(response);
        }

        let has_more = pages.is_some_and(|pages| current_page < pages);
        if !has_more {
            break;
        }
        if !req.all || pages_fetched >= req.max_pages {
            next_page = Some(current_page + 1);
            break;
        }
        current_page += 1;
    }

    let mut response = first_response.expect("at least one page fetched");
    if let Some(messages) = response.data.get_mut("messages") {
        if let Some(obj) = messages.as_object_mut() {
            obj.insert("matches".to_string(), json!(all_matches));
        }
    }

    let pagination = PaginationMeta {
        pages_fetched,
        next_cursor: None,
        next_page,
    };

    Ok((response, pagination))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_search_basic() {
        // This test requires a mock server to be implemented
        // For now, we just verify the function compiles
        let client = ApiClient::with_token("test_token".to_string());
        let result = search(&client, "test query".to_string(), None, None, None, None).await;
        // Result will fail because there's no mock server, but that's expected
        assert!(result.is_err());
    }
}
