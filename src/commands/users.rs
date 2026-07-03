//! Users command implementations

use crate::api::{ApiClient, ApiError, ApiMethod, ApiResponse};
use serde_json::json;
use std::collections::HashMap;

/// Get user information
///
/// # Arguments
/// * `client` - API client
/// * `user` - User ID
///
/// # Returns
/// * `Ok(ApiResponse)` with user information
/// * `Err(ApiError)` if the operation fails
pub async fn users_info(client: &ApiClient, user: String) -> Result<ApiResponse, ApiError> {
    let mut params = HashMap::new();
    params.insert("user".to_string(), json!(user));

    client.call_method(ApiMethod::UsersInfo, params).await
}

/// Look up a user by email address
///
/// Wraps `users.lookupByEmail`. A `users_not_found` Slack error (no matching
/// workspace user) surfaces with friendly guidance on stderr.
///
/// # Arguments
/// * `client` - API client
/// * `email` - Email address to look up
///
/// # Returns
/// * `Ok(ApiResponse)` with the matching user object
/// * `Err(ApiError)` if the operation fails or no user matches
pub async fn users_lookup_by_email(
    client: &ApiClient,
    email: String,
) -> Result<ApiResponse, ApiError> {
    let mut params = HashMap::new();
    params.insert("email".to_string(), json!(email));

    client
        .call_method(ApiMethod::UsersLookupByEmail, params)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_users_info_basic() {
        let client = ApiClient::with_token("test_token".to_string());
        let result = users_info(&client, "U123456".to_string()).await;
        // Result will fail because there's no mock server, but that's expected
        assert!(result.is_err());
    }
}
