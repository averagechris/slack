//! Integration tests for the wrapper-command feature set:
//! `conv open`, `users lookup --email`, `msg post --blocks`, and the
//! pagination flags (`--cursor` / `--all` / `--max-pages`) on
//! `conv history`, `thread get`, and `search`.

use httpmock::prelude::*;
use serde_json::json;
use serial_test::serial;
use slack::api::ApiClient;
use slack::commands;
use slack::commands::PageOptions;

fn client_for(server: &MockServer) -> ApiClient {
    ApiClient::new_with_base_url("test-token".to_string(), server.base_url())
}

// ---------------------------------------------------------------------------
// conv open (conversations.open)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_conv_open_single_user() {
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(POST)
            .path("/conversations.open")
            .header("Authorization", "Bearer test-token")
            .body_includes("\"users\":\"U123\"");
        then.status(200).json_body(json!({
            "ok": true,
            "channel": {"id": "D456"}
        }));
    });

    let client = client_for(&server);
    let response = commands::conv_open(&client, vec!["U123".to_string()])
        .await
        .unwrap();

    mock.assert();
    assert!(response.ok);
    assert_eq!(
        commands::extract_opened_channel_id(&response).unwrap(),
        "D456"
    );
}

#[tokio::test]
async fn test_conv_open_group_dm_joins_user_ids() {
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(POST)
            .path("/conversations.open")
            .body_includes("\"users\":\"U1,U2,U3\"");
        then.status(200).json_body(json!({
            "ok": true,
            "channel": {"id": "G789"}
        }));
    });

    let client = client_for(&server);
    let response = commands::conv_open(
        &client,
        vec!["U1".to_string(), "U2".to_string(), "U3".to_string()],
    )
    .await
    .unwrap();

    mock.assert();
    assert_eq!(
        commands::extract_opened_channel_id(&response).unwrap(),
        "G789"
    );
}

#[tokio::test]
async fn test_conv_open_user_not_found_error() {
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(POST).path("/conversations.open");
        then.status(200)
            .json_body(json!({"ok": false, "error": "user_not_found"}));
    });

    let client = client_for(&server);
    let result = commands::conv_open(&client, vec!["UNOPE".to_string()]).await;

    mock.assert();
    let err = result.unwrap_err().to_string();
    assert!(err.contains("user_not_found"), "got: {}", err);
}

#[test]
fn test_extract_opened_channel_id_missing_channel() {
    let response = slack::api::ApiResponse {
        ok: true,
        data: std::collections::HashMap::new(),
        error: None,
    };
    let err = commands::extract_opened_channel_id(&response).unwrap_err();
    assert!(err.contains("channel.id"), "got: {}", err);
}

// ---------------------------------------------------------------------------
// users lookup --email (users.lookupByEmail)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_users_lookup_by_email_happy_path() {
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(GET)
            .path("/users.lookupByEmail")
            .query_param("email", "alice@example.com");
        then.status(200).json_body(json!({
            "ok": true,
            "user": {"id": "U111", "name": "alice"}
        }));
    });

    let client = client_for(&server);
    let response = commands::users_lookup_by_email(&client, "alice@example.com".to_string())
        .await
        .unwrap();

    mock.assert();
    assert!(response.ok);
    assert_eq!(response.data["user"]["id"], "U111");
}

#[tokio::test]
async fn test_users_lookup_by_email_users_not_found() {
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(GET)
            .path("/users.lookupByEmail")
            .query_param("email", "ghost@example.com");
        then.status(200)
            .json_body(json!({"ok": false, "error": "users_not_found"}));
    });

    let client = client_for(&server);
    let result = commands::users_lookup_by_email(&client, "ghost@example.com".to_string()).await;

    mock.assert();
    let err = result.unwrap_err().to_string();
    assert!(err.contains("users_not_found"), "got: {}", err);
}

// ---------------------------------------------------------------------------
// msg post --blocks (chat.postMessage with Block Kit blocks)
// ---------------------------------------------------------------------------

#[tokio::test]
#[serial(write_guard)]
async fn test_msg_post_with_blocks_sends_blocks_and_fallback_text() {
    std::env::remove_var("SLACKCLI_ALLOW_WRITE");
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(POST)
            .path("/chat.postMessage")
            .body_includes("\"blocks\":[{")
            .body_includes("\"type\":\"section\"")
            .body_includes("\"text\":\"fallback\"");
        then.status(200).json_body(json!({"ok": true, "ts": "1.2"}));
    });

    let client = client_for(&server);
    let blocks: serde_json::Value =
        serde_json::from_str(r#"[{"type":"section","text":{"type":"mrkdwn","text":"*hi*"}}]"#)
            .unwrap();
    let result = commands::msg_post(
        &client,
        commands::MsgPostParams {
            channel: "C123".to_string(),
            text: Some("fallback".to_string()),
            blocks: Some(blocks),
            ..Default::default()
        },
        true,
        true,
    )
    .await;

    mock.assert();
    assert!(result.is_ok(), "msg post failed: {:?}", result.err());
}

#[tokio::test]
#[serial(write_guard)]
async fn test_msg_post_with_blocks_only_and_thread_ts() {
    std::env::remove_var("SLACKCLI_ALLOW_WRITE");
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(POST)
            .path("/chat.postMessage")
            .body_includes("\"blocks\":[{\"type\":\"divider\"}]")
            .body_includes("\"thread_ts\":\"1234567890.111111\"");
        then.status(200).json_body(json!({"ok": true, "ts": "1.3"}));
    });

    let client = client_for(&server);
    let result = commands::msg_post(
        &client,
        commands::MsgPostParams {
            channel: "C123".to_string(),
            text: None,
            blocks: Some(json!([{"type": "divider"}])),
            thread_ts: Some("1234567890.111111".to_string()),
            reply_broadcast: false,
        },
        true,
        true,
    )
    .await;

    mock.assert();
    assert!(result.is_ok(), "msg post failed: {:?}", result.err());
}

// ---------------------------------------------------------------------------
// conv history pagination (--cursor / --all / --max-pages)
// ---------------------------------------------------------------------------

fn history_page(messages: Vec<serde_json::Value>, next_cursor: &str) -> serde_json::Value {
    json!({
        "ok": true,
        "messages": messages,
        "has_more": !next_cursor.is_empty(),
        "response_metadata": {"next_cursor": next_cursor}
    })
}

#[tokio::test]
async fn test_conv_history_single_page_reports_next_cursor() {
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(GET)
            .path("/conversations.history")
            .query_param("channel", "C123")
            .query_param_missing("cursor");
        then.status(200)
            .json_body(history_page(vec![json!({"text": "m1"})], "cursor-1"));
    });

    let client = client_for(&server);
    let (response, pagination) = commands::conv_history_paged(
        &client,
        "C123".to_string(),
        None,
        None,
        None,
        &PageOptions::default(),
    )
    .await
    .unwrap();

    mock.assert();
    assert_eq!(pagination.pages_fetched, 1);
    assert_eq!(pagination.next_cursor.as_deref(), Some("cursor-1"));
    let messages = response.data["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 1);
    // Raw-friendly: aggregated response keeps the resume cursor
    assert_eq!(
        response.data["response_metadata"]["next_cursor"],
        "cursor-1"
    );
}

#[tokio::test]
async fn test_conv_history_all_follows_cursors_until_exhausted() {
    let server = MockServer::start();
    let page1 = server.mock(|when, then| {
        when.method(GET)
            .path("/conversations.history")
            .query_param("channel", "C123")
            .query_param_missing("cursor");
        then.status(200)
            .json_body(history_page(vec![json!({"text": "m1"})], "cursor-1"));
    });
    let page2 = server.mock(|when, then| {
        when.method(GET)
            .path("/conversations.history")
            .query_param("cursor", "cursor-1");
        then.status(200)
            .json_body(history_page(vec![json!({"text": "m2"})], "cursor-2"));
    });
    let page3 = server.mock(|when, then| {
        when.method(GET)
            .path("/conversations.history")
            .query_param("cursor", "cursor-2");
        then.status(200)
            .json_body(history_page(vec![json!({"text": "m3"})], ""));
    });

    let client = client_for(&server);
    let (response, pagination) = commands::conv_history_paged(
        &client,
        "C123".to_string(),
        None,
        None,
        None,
        &PageOptions {
            cursor: None,
            all: true,
            max_pages: 10,
        },
    )
    .await
    .unwrap();

    page1.assert();
    page2.assert();
    page3.assert();
    assert_eq!(pagination.pages_fetched, 3);
    assert!(pagination.next_cursor.is_none());
    let messages = response.data["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 3);
    assert_eq!(messages[2]["text"], "m3");
    assert_eq!(response.data["response_metadata"]["next_cursor"], "");
    assert_eq!(response.data["has_more"], false);
}

#[tokio::test]
async fn test_conv_history_all_respects_max_pages_cap() {
    let server = MockServer::start();
    let page1 = server.mock(|when, then| {
        when.method(GET)
            .path("/conversations.history")
            .query_param_missing("cursor");
        then.status(200)
            .json_body(history_page(vec![json!({"text": "m1"})], "cursor-1"));
    });
    let page2 = server.mock(|when, then| {
        when.method(GET)
            .path("/conversations.history")
            .query_param("cursor", "cursor-1");
        then.status(200)
            .json_body(history_page(vec![json!({"text": "m2"})], "cursor-2"));
    });

    let client = client_for(&server);
    let (response, pagination) = commands::conv_history_paged(
        &client,
        "C123".to_string(),
        None,
        None,
        None,
        &PageOptions {
            cursor: None,
            all: true,
            max_pages: 2,
        },
    )
    .await
    .unwrap();

    page1.assert();
    page2.assert();
    assert_eq!(pagination.pages_fetched, 2);
    assert_eq!(pagination.next_cursor.as_deref(), Some("cursor-2"));
    assert_eq!(response.data["messages"].as_array().unwrap().len(), 2);
    assert_eq!(
        response.data["response_metadata"]["next_cursor"],
        "cursor-2"
    );
}

#[tokio::test]
async fn test_conv_history_starts_from_explicit_cursor() {
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(GET)
            .path("/conversations.history")
            .query_param("cursor", "resume-here");
        then.status(200)
            .json_body(history_page(vec![json!({"text": "m9"})], ""));
    });

    let client = client_for(&server);
    let (response, pagination) = commands::conv_history_paged(
        &client,
        "C123".to_string(),
        None,
        None,
        None,
        &PageOptions::single_page(Some("resume-here".to_string())),
    )
    .await
    .unwrap();

    mock.assert();
    assert_eq!(pagination.pages_fetched, 1);
    assert!(pagination.next_cursor.is_none());
    assert_eq!(
        response.data["messages"].as_array().unwrap()[0]["text"],
        "m9"
    );
}

#[tokio::test]
async fn test_conv_history_paged_error_path() {
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(GET).path("/conversations.history");
        then.status(200)
            .json_body(json!({"ok": false, "error": "channel_not_found"}));
    });

    let client = client_for(&server);
    let result = commands::conv_history_paged(
        &client,
        "CNOPE".to_string(),
        None,
        None,
        None,
        &PageOptions {
            cursor: None,
            all: true,
            max_pages: 10,
        },
    )
    .await;

    mock.assert();
    let err = result.unwrap_err().to_string();
    assert!(err.contains("channel_not_found"), "got: {}", err);
}

// ---------------------------------------------------------------------------
// thread get pagination (--cursor / --all / --max-pages)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_thread_get_paged_single_page_by_default() {
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(GET)
            .path("/conversations.replies")
            .query_param("channel", "C123")
            .query_param("ts", "1.0")
            .query_param_missing("cursor");
        then.status(200)
            .json_body(history_page(vec![json!({"text": "r1"})], "cursor-1"));
    });

    let client = client_for(&server);
    let (response, pagination) = commands::thread_get_paged(
        &client,
        "C123".to_string(),
        "1.0".to_string(),
        None,
        None,
        &PageOptions::default(),
    )
    .await
    .unwrap();

    mock.assert();
    assert_eq!(pagination.pages_fetched, 1);
    assert_eq!(pagination.next_cursor.as_deref(), Some("cursor-1"));
    assert_eq!(response.data["messages"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn test_thread_get_paged_all_respects_max_pages() {
    let server = MockServer::start();
    let page1 = server.mock(|when, then| {
        when.method(GET)
            .path("/conversations.replies")
            .query_param_missing("cursor");
        then.status(200)
            .json_body(history_page(vec![json!({"text": "r1"})], "cursor-1"));
    });
    let page2 = server.mock(|when, then| {
        when.method(GET)
            .path("/conversations.replies")
            .query_param("cursor", "cursor-1");
        then.status(200)
            .json_body(history_page(vec![json!({"text": "r2"})], "cursor-2"));
    });

    let client = client_for(&server);
    let (response, pagination) = commands::thread_get_paged(
        &client,
        "C123".to_string(),
        "1.0".to_string(),
        None,
        None,
        &PageOptions {
            cursor: None,
            all: true,
            max_pages: 2,
        },
    )
    .await
    .unwrap();

    page1.assert();
    page2.assert();
    assert_eq!(pagination.pages_fetched, 2);
    assert_eq!(pagination.next_cursor.as_deref(), Some("cursor-2"));
    assert_eq!(response.data["messages"].as_array().unwrap().len(), 2);
}

// ---------------------------------------------------------------------------
// search --all (page-based pagination)
// ---------------------------------------------------------------------------

fn search_page(matches: Vec<serde_json::Value>, page: u32, pages: u32) -> serde_json::Value {
    json!({
        "ok": true,
        "query": "q",
        "messages": {
            "total": 3,
            "matches": matches,
            "paging": {"count": 1, "total": 3, "page": page, "pages": pages}
        }
    })
}

#[tokio::test]
async fn test_search_paged_all_merges_matches_across_pages() {
    let server = MockServer::start();
    let page1 = server.mock(|when, then| {
        when.method(GET)
            .path("/search.messages")
            .query_param("query", "q")
            .query_param("page", "1");
        then.status(200)
            .json_body(search_page(vec![json!({"ts": "1"})], 1, 2));
    });
    let page2 = server.mock(|when, then| {
        when.method(GET)
            .path("/search.messages")
            .query_param("query", "q")
            .query_param("page", "2");
        then.status(200)
            .json_body(search_page(vec![json!({"ts": "2"})], 2, 2));
    });

    let client = client_for(&server);
    let (response, pagination) = commands::search_paged(
        &client,
        &commands::SearchRequest {
            query: "q".to_string(),
            count: None,
            page: None,
            sort: None,
            sort_dir: None,
            all: true,
            max_pages: 10,
        },
    )
    .await
    .unwrap();

    page1.assert();
    page2.assert();
    assert_eq!(pagination.pages_fetched, 2);
    assert!(pagination.next_page.is_none());
    let matches = response.data["messages"]["matches"].as_array().unwrap();
    assert_eq!(matches.len(), 2);
    assert_eq!(matches[1]["ts"], "2");
}

#[tokio::test]
async fn test_search_paged_all_respects_max_pages_and_reports_next_page() {
    let server = MockServer::start();
    let page1 = server.mock(|when, then| {
        when.method(GET)
            .path("/search.messages")
            .query_param("page", "1");
        then.status(200)
            .json_body(search_page(vec![json!({"ts": "1"})], 1, 3));
    });
    let page2 = server.mock(|when, then| {
        when.method(GET)
            .path("/search.messages")
            .query_param("page", "2");
        then.status(200)
            .json_body(search_page(vec![json!({"ts": "2"})], 2, 3));
    });

    let client = client_for(&server);
    let (response, pagination) = commands::search_paged(
        &client,
        &commands::SearchRequest {
            query: "q".to_string(),
            count: None,
            page: None,
            sort: None,
            sort_dir: None,
            all: true,
            max_pages: 2,
        },
    )
    .await
    .unwrap();

    page1.assert();
    page2.assert();
    assert_eq!(pagination.pages_fetched, 2);
    assert_eq!(pagination.next_page, Some(3));
    assert_eq!(
        response.data["messages"]["matches"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

#[tokio::test]
async fn test_search_paged_single_page_reports_next_page() {
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(GET)
            .path("/search.messages")
            .query_param("page", "1");
        then.status(200)
            .json_body(search_page(vec![json!({"ts": "1"})], 1, 5));
    });

    let client = client_for(&server);
    let (_, pagination) = commands::search_paged(
        &client,
        &commands::SearchRequest {
            query: "q".to_string(),
            count: None,
            page: None,
            sort: None,
            sort_dir: None,
            all: false,
            max_pages: 10,
        },
    )
    .await
    .unwrap();

    mock.assert();
    assert_eq!(pagination.pages_fetched, 1);
    assert_eq!(pagination.next_page, Some(2));
}

#[tokio::test]
async fn test_search_paged_error_path() {
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(GET).path("/search.messages");
        then.status(200)
            .json_body(json!({"ok": false, "error": "not_allowed_token_type"}));
    });

    let client = client_for(&server);
    let result = commands::search_paged(
        &client,
        &commands::SearchRequest {
            query: "q".to_string(),
            count: None,
            page: None,
            sort: None,
            sort_dir: None,
            all: true,
            max_pages: 10,
        },
    )
    .await;

    mock.assert();
    let err = result.unwrap_err().to_string();
    assert!(err.contains("not_allowed_token_type"), "got: {}", err);
}
