//! HTTP client for Slack API calls
//!
//! This module provides a configurable HTTP client with:
//! - Configurable base URL (for testing with mock servers)
//! - Retry logic with exponential backoff
//! - Rate limit handling (429 + Retry-After)
//! - Support for both wrapper commands and generic API calls

use reqwest::{Client, Method, Response, StatusCode};
use serde_json::Value;
use std::collections::HashMap;
use std::time::Duration;
use thiserror::Error;

use super::guidance::format_error_guidance;
use super::types::{ApiMethod, ApiResponse};

/// API client errors (for wrapper commands)
#[derive(Error, Debug)]
pub enum ApiError {
    #[error("HTTP request failed: {0}")]
    RequestFailed(#[from] reqwest::Error),

    #[error("JSON serialization failed: {0}")]
    JsonError(#[from] serde_json::Error),

    #[error("Slack API error: {0}")]
    SlackError(String),

    #[allow(dead_code)]
    #[error("Missing required parameter: {0}")]
    MissingParameter(String),

    #[error("Write operation denied. Set SLACKCLI_ALLOW_WRITE=true to enable write operations")]
    WriteNotAllowed,

    #[error("Destructive operation cancelled")]
    OperationCancelled,

    #[error("Non-interactive mode error: {0}")]
    NonInteractiveError(String),
}

/// API client errors (for generic API calls)
#[derive(Debug, Error)]
pub enum ApiClientError {
    #[error("HTTP request failed: {0}")]
    RequestFailed(#[from] reqwest::Error),

    #[error("Rate limit exceeded, retry after {0} seconds")]
    RateLimitExceeded(u64),

    #[error("API error: {0}")]
    ApiError(String),

    #[error("Invalid response: {0}")]
    InvalidResponse(String),
}

pub type Result<T> = std::result::Result<T, ApiClientError>;

/// Configuration for the API client
#[derive(Debug, Clone)]
pub struct ApiClientConfig {
    /// Base URL for API calls (default: https://slack.com/api)
    pub base_url: String,

    /// Maximum number of retry attempts
    pub max_retries: u32,

    /// Initial backoff duration in milliseconds
    pub initial_backoff_ms: u64,

    /// Maximum backoff duration in milliseconds
    pub max_backoff_ms: u64,
}

impl Default for ApiClientConfig {
    fn default() -> Self {
        Self {
            base_url: "https://slack.com/api".to_string(),
            max_retries: 3,
            initial_backoff_ms: 1000,
            max_backoff_ms: 32000,
        }
    }
}

/// Slack API client
///
/// Supports both:
/// - Wrapper commands via `call_method()` with `ApiMethod` enum
/// - Generic API calls via `call()` with arbitrary endpoints
pub struct ApiClient {
    client: Client,
    pub(crate) token: Option<String>,
    config: ApiClientConfig,
}

impl ApiClient {
    /// Create a new API client with default configuration (for generic API calls)
    pub fn new() -> Self {
        Self::with_config(ApiClientConfig::default())
    }

    /// Create a new API client with a token (for wrapper commands)
    pub fn with_token(token: String) -> Self {
        Self {
            client: Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .expect("Failed to create HTTP client"),
            token: Some(token),
            config: ApiClientConfig::default(),
        }
    }

    /// Create a new API client with custom configuration
    pub fn with_config(config: ApiClientConfig) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("Failed to create HTTP client");

        Self {
            client,
            token: None,
            config,
        }
    }

    /// Create a new API client with custom base URL (for testing)
    #[doc(hidden)]
    #[allow(dead_code)]
    pub fn new_with_base_url(token: String, base_url: String) -> Self {
        Self {
            client: Client::new(),
            token: Some(token),
            config: ApiClientConfig {
                base_url,
                ..Default::default()
            },
        }
    }

    /// Get the base URL
    pub fn base_url(&self) -> &str {
        &self.config.base_url
    }

    /// Call a Slack API method using the ApiMethod enum (for wrapper commands)
    ///
    /// Uses the same 429/Retry-After + exponential backoff retry behavior as
    /// the generic `call()` path.
    pub async fn call_method(
        &self,
        method: ApiMethod,
        params: HashMap<String, Value>,
    ) -> std::result::Result<ApiResponse, ApiError> {
        let token = self
            .token
            .as_ref()
            .ok_or_else(|| ApiError::SlackError("No token configured".to_string()))?;

        let url = format!("{}/{}", self.config.base_url, method.as_str());

        // Prepare request inputs once so each retry attempt builds a fresh request
        let query_params: Option<Vec<(String, String)>> = if method.uses_get_method() {
            let mut qp = vec![];
            for (key, value) in &params {
                let value_str = match value {
                    Value::String(s) => s.clone(),
                    Value::Number(n) => n.to_string(),
                    Value::Bool(b) => b.to_string(),
                    _ => serde_json::to_string(value).unwrap_or_default(),
                };
                qp.push((key.clone(), value_str));
            }
            Some(qp)
        } else {
            None
        };

        let response = self
            .send_with_retry(|| match &query_params {
                Some(qp) => self.client.get(&url).bearer_auth(token).query(qp),
                None => self.client.post(&url).bearer_auth(token).json(&params),
            })
            .await
            .map_err(|e| match e {
                ApiClientError::RequestFailed(e) => ApiError::RequestFailed(e),
                ApiClientError::RateLimitExceeded(secs) => {
                    ApiError::SlackError(format!("rate_limited: retry after {} seconds", secs))
                }
                other => ApiError::SlackError(other.to_string()),
            })?;

        let response_json: ApiResponse = response.json().await?;

        if !response_json.ok {
            let error_code = response_json.error.as_deref().unwrap_or("Unknown error");

            // Display error guidance if available
            if let Some(guidance) = format_error_guidance(error_code) {
                eprintln!("{}", guidance);
            }

            return Err(ApiError::SlackError(error_code.to_string()));
        }

        Ok(response_json)
    }

    /// Make an API call with automatic retry logic (for generic API calls)
    pub async fn call(
        &self,
        method: Method,
        endpoint: &str,
        token: &str,
        body: RequestBody,
        query_params: Vec<(String, String)>,
    ) -> Result<Response> {
        let url = format!("{}/{}", self.config.base_url, endpoint);

        self.send_with_retry(|| self.build_request(&url, &method, token, &body, &query_params))
            .await
    }

    /// Send a request with 429/Retry-After handling and exponential backoff
    ///
    /// Shared retry loop used by both `call()` (generic API calls) and
    /// `call_method()` (wrapper commands). `build_request` is invoked once per
    /// attempt to construct a fresh request.
    async fn send_with_retry<F>(&self, build_request: F) -> Result<Response>
    where
        F: Fn() -> reqwest::RequestBuilder,
    {
        let mut attempt = 0;

        loop {
            let response = build_request().send().await?;

            // Check for rate limiting
            if response.status() == StatusCode::TOO_MANY_REQUESTS {
                // Extract Retry-After header
                let retry_after = self.extract_retry_after(&response);

                if attempt >= self.config.max_retries {
                    return Err(ApiClientError::RateLimitExceeded(retry_after));
                }

                // Wait for the specified duration
                tokio::time::sleep(Duration::from_secs(retry_after)).await;
                attempt += 1;
                continue;
            }

            // For other errors, apply exponential backoff
            if !response.status().is_success() && attempt < self.config.max_retries {
                let backoff = self.calculate_backoff(attempt);
                tokio::time::sleep(backoff).await;
                attempt += 1;
                continue;
            }

            return Ok(response);
        }
    }

    /// Build a single HTTP request (one retry attempt)
    fn build_request(
        &self,
        url: &str,
        method: &Method,
        token: &str,
        body: &RequestBody,
        query_params: &[(String, String)],
    ) -> reqwest::RequestBuilder {
        let mut request = self.client.request(method.clone(), url);

        // Add authorization header
        request = request.header("Authorization", format!("Bearer {}", token));

        // Add query parameters
        if !query_params.is_empty() {
            request = request.query(query_params);
        }

        // Add body based on type
        match body {
            RequestBody::Form(params) => {
                request = request
                    .header("Content-Type", "application/x-www-form-urlencoded")
                    .form(params);
            }
            RequestBody::Json(json) => {
                request = request
                    .header("Content-Type", "application/json")
                    .json(json);
            }
            RequestBody::None => {}
        }

        request
    }

    /// Extract Retry-After header value
    fn extract_retry_after(&self, response: &Response) -> u64 {
        response
            .headers()
            .get("Retry-After")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(60) // Default to 60 seconds if not specified
    }

    /// Calculate exponential backoff with jitter
    fn calculate_backoff(&self, attempt: u32) -> Duration {
        let base = self.config.initial_backoff_ms;
        let max = self.config.max_backoff_ms;

        // Exponential backoff: base * 2^attempt
        let backoff = base * 2_u64.pow(attempt);
        let backoff = backoff.min(max);

        // Add jitter (±25%)
        let jitter = (backoff as f64 * 0.25) as u64;
        let jitter = rand::random::<u64>() % (jitter * 2 + 1);
        let backoff = backoff
            .saturating_sub(jitter / 2)
            .saturating_add(jitter / 2);

        Duration::from_millis(backoff)
    }
}

impl Default for ApiClient {
    fn default() -> Self {
        Self::new()
    }
}

/// Request body type
#[derive(Debug, Clone)]
pub enum RequestBody {
    Form(Vec<(String, String)>),
    Json(Value),
    None,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_api_method_as_str() {
        assert_eq!(ApiMethod::SearchMessages.as_str(), "search.messages");
        assert_eq!(ApiMethod::ConversationsList.as_str(), "conversations.list");
        assert_eq!(
            ApiMethod::ConversationsHistory.as_str(),
            "conversations.history"
        );
        assert_eq!(
            ApiMethod::ConversationsReplies.as_str(),
            "conversations.replies"
        );
        assert_eq!(ApiMethod::UsersInfo.as_str(), "users.info");
        assert_eq!(ApiMethod::ChatPostMessage.as_str(), "chat.postMessage");
        assert_eq!(ApiMethod::ChatUpdate.as_str(), "chat.update");
        assert_eq!(ApiMethod::ChatDelete.as_str(), "chat.delete");
        assert_eq!(ApiMethod::ReactionsAdd.as_str(), "reactions.add");
        assert_eq!(ApiMethod::ReactionsRemove.as_str(), "reactions.remove");
    }

    #[test]
    fn test_api_method_is_write() {
        assert!(!ApiMethod::SearchMessages.is_write());
        assert!(!ApiMethod::ConversationsList.is_write());
        assert!(!ApiMethod::ConversationsHistory.is_write());
        assert!(!ApiMethod::UsersInfo.is_write());
        assert!(ApiMethod::ChatPostMessage.is_write());
        assert!(ApiMethod::ChatUpdate.is_write());
        assert!(ApiMethod::ChatDelete.is_write());
        assert!(ApiMethod::ReactionsAdd.is_write());
        assert!(ApiMethod::ReactionsRemove.is_write());
    }

    #[test]
    fn test_api_method_is_destructive() {
        assert!(!ApiMethod::SearchMessages.is_destructive());
        assert!(!ApiMethod::ConversationsList.is_destructive());
        assert!(!ApiMethod::ConversationsHistory.is_destructive());
        assert!(!ApiMethod::UsersInfo.is_destructive());
        assert!(!ApiMethod::ChatPostMessage.is_destructive());
        assert!(ApiMethod::ChatUpdate.is_destructive());
        assert!(ApiMethod::ChatDelete.is_destructive());
        assert!(!ApiMethod::ReactionsAdd.is_destructive());
        assert!(ApiMethod::ReactionsRemove.is_destructive());
    }

    #[test]
    fn test_api_method_uses_get() {
        // GET methods
        assert!(ApiMethod::SearchMessages.uses_get_method());
        assert!(ApiMethod::ConversationsList.uses_get_method());
        assert!(ApiMethod::ConversationsHistory.uses_get_method());
        assert!(ApiMethod::ConversationsReplies.uses_get_method());
        assert!(ApiMethod::UsersInfo.uses_get_method());
        assert!(ApiMethod::UsersList.uses_get_method());

        // POST methods
        assert!(!ApiMethod::ChatPostMessage.uses_get_method());
        assert!(!ApiMethod::ChatUpdate.uses_get_method());
        assert!(!ApiMethod::ChatDelete.uses_get_method());
        assert!(!ApiMethod::ReactionsAdd.uses_get_method());
        assert!(!ApiMethod::ReactionsRemove.uses_get_method());
    }

    #[test]
    fn test_api_client_config_default() {
        let config = ApiClientConfig::default();
        assert_eq!(config.base_url, "https://slack.com/api");
        assert_eq!(config.max_retries, 3);
        assert_eq!(config.initial_backoff_ms, 1000);
        assert_eq!(config.max_backoff_ms, 32000);
    }

    #[test]
    fn test_api_client_creation() {
        let client = ApiClient::new();
        assert_eq!(client.base_url(), "https://slack.com/api");
    }

    #[test]
    fn test_api_client_custom_config() {
        let config = ApiClientConfig {
            base_url: "https://test.example.com".to_string(),
            max_retries: 5,
            initial_backoff_ms: 500,
            max_backoff_ms: 10000,
        };

        let client = ApiClient::with_config(config.clone());
        assert_eq!(client.base_url(), "https://test.example.com");
        assert_eq!(client.config.max_retries, 5);
    }

    mod retry_tests {
        use super::*;
        use wiremock::matchers::{method as http_method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        #[tokio::test]
        async fn test_call_method_retries_on_429_then_succeeds() {
            let server = MockServer::start().await;

            // First request: rate limited
            Mock::given(http_method("POST"))
                .and(path("/chat.postMessage"))
                .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "0"))
                .up_to_n_times(1)
                .expect(1)
                .mount(&server)
                .await;

            // Subsequent request: success
            Mock::given(http_method("POST"))
                .and(path("/chat.postMessage"))
                .respond_with(
                    ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok": true})),
                )
                .expect(1)
                .mount(&server)
                .await;

            let client = ApiClient::new_with_base_url("xoxb-test".to_string(), server.uri());
            let mut params = HashMap::new();
            params.insert("channel".to_string(), Value::String("C123".to_string()));
            params.insert("text".to_string(), Value::String("hello".to_string()));

            let response = client
                .call_method(ApiMethod::ChatPostMessage, params)
                .await
                .expect("call_method should retry after 429 and succeed");
            assert!(response.ok);
        }

        #[tokio::test]
        async fn test_call_method_get_retries_on_429_then_succeeds() {
            let server = MockServer::start().await;

            Mock::given(http_method("GET"))
                .and(path("/users.info"))
                .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "0"))
                .up_to_n_times(1)
                .expect(1)
                .mount(&server)
                .await;

            Mock::given(http_method("GET"))
                .and(path("/users.info"))
                .respond_with(
                    ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok": true})),
                )
                .expect(1)
                .mount(&server)
                .await;

            let client = ApiClient::new_with_base_url("xoxb-test".to_string(), server.uri());
            let mut params = HashMap::new();
            params.insert("user".to_string(), Value::String("U123".to_string()));

            let response = client
                .call_method(ApiMethod::UsersInfo, params)
                .await
                .expect("GET call_method should retry after 429 and succeed");
            assert!(response.ok);
        }

        #[tokio::test]
        async fn test_call_method_429_retries_exhausted() {
            let server = MockServer::start().await;

            // Always rate limited
            Mock::given(http_method("POST"))
                .and(path("/chat.postMessage"))
                .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "0"))
                // initial attempt + max_retries (3) = 4 requests
                .expect(4)
                .mount(&server)
                .await;

            let client = ApiClient::new_with_base_url("xoxb-test".to_string(), server.uri());
            let mut params = HashMap::new();
            params.insert("channel".to_string(), Value::String("C123".to_string()));

            let result = client.call_method(ApiMethod::ChatPostMessage, params).await;
            match result {
                Err(ApiError::SlackError(msg)) => {
                    assert!(msg.contains("rate_limited"), "got: {}", msg);
                }
                other => panic!("Expected SlackError(rate_limited), got: {:?}", other.err()),
            }
        }

        #[tokio::test]
        async fn test_call_retries_on_429_then_succeeds() {
            let server = MockServer::start().await;

            Mock::given(http_method("POST"))
                .and(path("/chat.postMessage"))
                .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "0"))
                .up_to_n_times(1)
                .expect(1)
                .mount(&server)
                .await;

            Mock::given(http_method("POST"))
                .and(path("/chat.postMessage"))
                .respond_with(
                    ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok": true})),
                )
                .expect(1)
                .mount(&server)
                .await;

            let client = ApiClient::with_config(ApiClientConfig {
                base_url: server.uri(),
                ..Default::default()
            });

            let response = client
                .call(
                    Method::POST,
                    "chat.postMessage",
                    "xoxb-test",
                    RequestBody::Json(serde_json::json!({"channel": "C123"})),
                    vec![],
                )
                .await
                .expect("call should retry after 429 and succeed");
            assert_eq!(response.status(), StatusCode::OK);
        }

        #[tokio::test]
        async fn test_call_429_retries_exhausted_returns_rate_limit_error() {
            let server = MockServer::start().await;

            Mock::given(http_method("POST"))
                .and(path("/chat.postMessage"))
                .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "0"))
                .expect(4)
                .mount(&server)
                .await;

            let client = ApiClient::with_config(ApiClientConfig {
                base_url: server.uri(),
                ..Default::default()
            });

            let result = client
                .call(
                    Method::POST,
                    "chat.postMessage",
                    "xoxb-test",
                    RequestBody::None,
                    vec![],
                )
                .await;

            assert!(matches!(result, Err(ApiClientError::RateLimitExceeded(_))));
        }
    }
}
