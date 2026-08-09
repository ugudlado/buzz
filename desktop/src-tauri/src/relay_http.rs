//! Small, standalone additions to the relay HTTP bridge (`relay.rs`) — kept
//! in a separate file so `relay.rs` doesn't grow past the desktop file-size
//! ratchet. Add new one-off bridge calls here rather than in `relay.rs`.

use reqwest::Method;
use serde::de::DeserializeOwned;

use crate::app_state::AppState;
use crate::relay::{
    build_nip98_auth_header, classify_request_error, parse_json_response,
    relay_api_base_url_with_override, relay_error_message,
};

/// Execute an authenticated GET against the relay's HTTP bridge and
/// deserialize the JSON body.
///
/// `path` is the request path including any query string (e.g.
/// `/api/workflows/{id}/runs?limit=50`) — NIP-98 signs the full URL, so the
/// same string is used both to build the request and to compute the auth
/// header. Mirrors [`crate::relay::query_relay`]'s auth/error handling for a
/// plain GET instead of the `/query` POST bridge.
pub async fn get_relay_json<T: DeserializeOwned>(
    state: &AppState,
    path: &str,
) -> Result<T, String> {
    crate::relay_admission::wait_for_rate_limit().await;
    let url = format!("{}{}", relay_api_base_url_with_override(state), path);
    let auth = build_nip98_auth_header(&Method::GET, &url, &[], state)?;

    let response = state
        .http_client
        .get(&url)
        .header("Authorization", auth)
        .send()
        .await
        .map_err(|e| classify_request_error(&e))?;

    if !response.status().is_success() {
        return Err(relay_error_message(response).await);
    }

    parse_json_response(response).await
}
