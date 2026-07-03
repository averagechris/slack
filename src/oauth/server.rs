//! Local callback server for OAuth flow
//!
//! Runs a temporary HTTP server on localhost to receive the OAuth callback

use super::types::OAuthError;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::time::{timeout, Duration};

#[derive(Debug, Clone)]
pub struct CallbackResult {
    pub code: String,
    #[allow(dead_code)]
    pub state: String,
}

/// Run a local HTTP server to receive OAuth callback
///
/// Returns the authorization code and state received from the callback
///
/// Requests carrying a wrong `state` value are rejected with an error page
/// but do NOT abort the login: the server keeps waiting for the correct
/// callback until the timeout expires. This prevents a trivial DoS when the
/// callback endpoint is reachable by third parties (e.g. via a tunnel).
///
/// # Arguments
/// * `port` - Port to listen on (typically 3000)
/// * `expected_state` - Expected state value for CSRF verification
/// * `timeout_secs` - Timeout in seconds (default 300)
pub async fn run_callback_server(
    port: u16,
    expected_state: String,
    timeout_secs: u64,
) -> Result<CallbackResult, OAuthError> {
    let bind_addr = format!("127.0.0.1:{}", port);
    let listener = TcpListener::bind(&bind_addr)
        .await
        .map_err(|e| OAuthError::ServerError(format!("Failed to bind to port {}: {}", port, e)))?;

    let actual_port = listener.local_addr().map(|a| a.port()).unwrap_or(port);
    println!(
        "Listening for OAuth callback on http://127.0.0.1:{}",
        actual_port
    );

    let result: Arc<Mutex<Option<Result<CallbackResult, OAuthError>>>> = Arc::new(Mutex::new(None));

    let server_result = result.clone();
    let server_task = async move {
        loop {
            let (mut socket, _) = match listener.accept().await {
                Ok(conn) => conn,
                Err(e) => {
                    let mut res = server_result.lock().unwrap();
                    *res = Some(Err(OAuthError::ServerError(format!(
                        "Failed to accept connection: {}",
                        e
                    ))));
                    break;
                }
            };

            let mut buffer = vec![0; 4096];
            let n = match socket.read(&mut buffer).await {
                Ok(n) if n > 0 => n,
                _ => continue,
            };

            let request = String::from_utf8_lossy(&buffer[..n]);

            // Parse the request line
            let path_part = match request
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
            {
                Some(p) => p,
                None => continue,
            };

            let query = match path_part.find('?') {
                Some(query_start) => &path_part[query_start + 1..],
                None => {
                    // Not a callback request (e.g. favicon); reject and keep waiting
                    let response = create_error_response("Missing required parameters");
                    let _ = socket.write_all(response.as_bytes()).await;
                    let _ = socket.flush().await;
                    continue;
                }
            };

            let params = parse_query_string(query);

            // Whether this request terminates the server loop
            let mut done = false;

            let response =
                if let (Some(code), Some(state)) = (params.get("code"), params.get("state")) {
                    // Verify state in constant time
                    if !constant_time_eq(state.as_bytes(), expected_state.as_bytes()) {
                        // Wrong state: reject this request but keep waiting for the
                        // legitimate callback (do not abort the login).
                        create_error_response("State mismatch - request rejected")
                    } else {
                        let mut res = server_result.lock().unwrap();
                        *res = Some(Ok(CallbackResult {
                            code: code.clone(),
                            state: state.clone(),
                        }));
                        done = true;
                        create_success_response()
                    }
                } else if let Some(error) = params.get("error") {
                    let mut res = server_result.lock().unwrap();
                    *res = Some(Err(OAuthError::SlackError(error.clone())));
                    done = true;
                    create_error_response(&format!("OAuth error: {}", error))
                } else {
                    // Missing parameters: reject and keep waiting
                    create_error_response("Missing required parameters")
                };

            let _ = socket.write_all(response.as_bytes()).await;
            let _ = socket.flush().await;

            if done {
                break;
            }
        }
    };

    // Run with timeout
    match timeout(Duration::from_secs(timeout_secs), server_task).await {
        Ok(_) => {
            let res = result.lock().unwrap();
            match res.as_ref() {
                Some(Ok(callback_result)) => Ok(callback_result.clone()),
                Some(Err(e)) => Err(format_oauth_error(e)),
                None => Err(OAuthError::ServerError("No result received".to_string())),
            }
        }
        Err(_) => Err(OAuthError::ServerError(format!(
            "Timeout after {} seconds waiting for callback",
            timeout_secs
        ))),
    }
}

/// Constant-time byte-slice equality (length check + byte-wise OR-fold)
///
/// Note: the length comparison itself is not constant-time, which is
/// acceptable — the state length is public knowledge.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter()
        .zip(b.iter())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}

/// Helper function to format OAuthError for re-creation
fn format_oauth_error(err: &OAuthError) -> OAuthError {
    match err {
        OAuthError::ConfigError(msg) => OAuthError::ConfigError(msg.clone()),
        OAuthError::NetworkError(msg) => OAuthError::NetworkError(msg.clone()),
        OAuthError::HttpError(code, msg) => OAuthError::HttpError(*code, msg.clone()),
        OAuthError::ParseError(msg) => OAuthError::ParseError(msg.clone()),
        OAuthError::SlackError(msg) => OAuthError::SlackError(msg.clone()),
        OAuthError::StateMismatch => OAuthError::StateMismatch,
        OAuthError::ServerError(msg) => OAuthError::ServerError(msg.clone()),
        OAuthError::BrowserError(msg) => OAuthError::BrowserError(msg.clone()),
    }
}

/// Parse URL query string into a HashMap
fn parse_query_string(query: &str) -> HashMap<String, String> {
    query
        .split('&')
        .filter_map(|pair| {
            let mut parts = pair.split('=');
            match (parts.next(), parts.next()) {
                (Some(key), Some(value)) => Some((key.to_string(), urlencoding::decode(value)?)),
                _ => None,
            }
        })
        .collect()
}

fn create_success_response() -> String {
    "HTTP/1.1 200 OK\r\n\
     Content-Type: text/html; charset=utf-8\r\n\
     Connection: close\r\n\
     \r\n\
     <html>\
     <head><title>Authentication Successful</title></head>\
     <body>\
     <h1>✓ Authentication Successful</h1>\
     <p>You can close this window and return to the CLI.</p>\
     </body>\
     </html>"
        .to_string()
}

fn create_error_response(message: &str) -> String {
    format!(
        "HTTP/1.1 400 Bad Request\r\n\
         Content-Type: text/html; charset=utf-8\r\n\
         Connection: close\r\n\
         \r\n\
         <html>\
         <head><title>Authentication Failed</title></head>\
         <body>\
         <h1>✗ Authentication Failed</h1>\
         <p>{}</p>\
         </body>\
         </html>",
        message
    )
}

/// Minimal percent-decoding for query parameters (no external dependency)
mod urlencoding {
    /// Decode a percent-encoded string, treating `+` as space.
    ///
    /// Decodes into a byte buffer first so multibyte UTF-8 sequences are
    /// reassembled correctly, then validates the result as UTF-8. Returns
    /// `None` for malformed percent-escapes or invalid UTF-8.
    pub fn decode(s: &str) -> Option<String> {
        let mut bytes: Vec<u8> = Vec::with_capacity(s.len());
        let mut input = s.as_bytes().iter();
        while let Some(&b) = input.next() {
            match b {
                b'%' => {
                    let hi = *input.next()?;
                    let lo = *input.next()?;
                    let hex = [hi, lo];
                    let hex_str = std::str::from_utf8(&hex).ok()?;
                    let byte = u8::from_str_radix(hex_str, 16).ok()?;
                    bytes.push(byte);
                }
                b'+' => bytes.push(b' '),
                other => bytes.push(other),
            }
        }
        String::from_utf8(bytes).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpStream;

    #[test]
    fn test_parse_query_string() {
        let query = "code=test_code&state=test_state&foo=bar";
        let params = parse_query_string(query);

        assert_eq!(params.get("code"), Some(&"test_code".to_string()));
        assert_eq!(params.get("state"), Some(&"test_state".to_string()));
        assert_eq!(params.get("foo"), Some(&"bar".to_string()));
    }

    #[test]
    fn test_parse_query_string_with_encoding() {
        let query = "message=hello+world&name=test%20user";
        let params = parse_query_string(query);

        assert_eq!(params.get("message"), Some(&"hello world".to_string()));
        assert_eq!(params.get("name"), Some(&"test user".to_string()));
    }

    #[test]
    fn test_parse_query_string_multibyte_utf8() {
        // "あ" (U+3042) percent-encoded as UTF-8, and "é" (U+00E9)
        let query = "name=%E3%81%82&city=caf%C3%A9";
        let params = parse_query_string(query);

        assert_eq!(params.get("name"), Some(&"あ".to_string()));
        assert_eq!(params.get("city"), Some(&"café".to_string()));
    }

    #[test]
    fn test_urlencoding_decode_invalid_utf8_is_rejected() {
        // 0xFF is never valid in UTF-8
        assert_eq!(urlencoding::decode("%FF"), None);
        // Truncated escape
        assert_eq!(urlencoding::decode("%E3%8"), None);
        // Non-hex escape
        assert_eq!(urlencoding::decode("%ZZ"), None);
    }

    #[test]
    fn test_constant_time_eq() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
        assert!(constant_time_eq(b"", b""));
    }

    #[tokio::test]
    async fn test_callback_server_timeout() {
        // Test that the server times out appropriately
        let state = "test_state".to_string();
        // Use an ephemeral port to avoid test flakiness from port conflicts.
        let result = run_callback_server(0, state, 1).await;

        assert!(result.is_err());
        match result {
            Err(OAuthError::ServerError(msg)) => {
                assert!(msg.contains("Timeout"));
            }
            _ => panic!("Expected ServerError with timeout"),
        }
    }

    /// Find a free localhost port by binding to port 0 and dropping the listener.
    async fn free_port() -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        listener.local_addr().unwrap().port()
    }

    async fn send_request(port: u16, path: &str) -> String {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        let request = format!("GET {} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n", path);
        stream.write_all(request.as_bytes()).await.unwrap();
        stream.flush().await.unwrap();
        let mut response = Vec::new();
        let _ = stream.read_to_end(&mut response).await;
        String::from_utf8_lossy(&response).to_string()
    }

    #[tokio::test]
    async fn test_wrong_state_then_correct_state_succeeds() {
        let port = free_port().await;
        let expected_state = "correct_state".to_string();

        let server = tokio::spawn(run_callback_server(port, expected_state, 10));

        // Give the server a moment to bind
        tokio::time::sleep(Duration::from_millis(100)).await;

        // 1. Wrong-state request: must be rejected but NOT abort the login
        let response = send_request(port, "/callback?code=evil_code&state=wrong_state").await;
        assert!(response.contains("400 Bad Request"), "got: {}", response);
        // Error page must not leak the expected state value
        assert!(!response.contains("correct_state"));

        // 2. Correct-state request: must succeed
        let response = send_request(port, "/callback?code=good_code&state=correct_state").await;
        assert!(response.contains("200 OK"), "got: {}", response);

        let result = server.await.unwrap().unwrap();
        assert_eq!(result.code, "good_code");
        assert_eq!(result.state, "correct_state");
    }

    #[tokio::test]
    async fn test_missing_params_then_correct_state_succeeds() {
        let port = free_port().await;
        let expected_state = "state123".to_string();

        let server = tokio::spawn(run_callback_server(port, expected_state, 10));
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Request with query but no code/state: rejected, server keeps waiting
        let response = send_request(port, "/callback?foo=bar").await;
        assert!(response.contains("400 Bad Request"));

        // Correct callback still succeeds
        let response = send_request(port, "/callback?code=abc&state=state123").await;
        assert!(response.contains("200 OK"));

        let result = server.await.unwrap().unwrap();
        assert_eq!(result.code, "abc");
    }
}
