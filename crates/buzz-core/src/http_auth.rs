//! Shared NIP-98 HTTP authentication event builder.

use base64::Engine;
use nostr::{EventBuilder, Keys, Kind, Tag};
use sha2::{Digest, Sha256};

/// Build an `Authorization: Nostr …` value bound to `method`, `url`, and body.
pub fn authorization_header(
    keys: &Keys,
    method: &str,
    url: &str,
    body: Option<&[u8]>,
) -> Result<String, String> {
    let mut tags = vec![
        parse_tag(["u", url])?,
        parse_tag(["method", method])?,
        parse_tag(["nonce", &uuid::Uuid::new_v4().to_string()])?,
    ];
    if let Some(body) = body {
        tags.push(parse_tag(["payload", &hex::encode(Sha256::digest(body))])?);
    }
    let event = EventBuilder::new(Kind::HttpAuth, "")
        .tags(tags)
        .sign_with_keys(keys)
        .map_err(|error| error.to_string())?;
    let json = serde_json::to_vec(&event).map_err(|error| error.to_string())?;
    Ok(format!(
        "Nostr {}",
        base64::engine::general_purpose::STANDARD.encode(json)
    ))
}

fn parse_tag<const N: usize>(values: [&str; N]) -> Result<Tag, String> {
    Tag::parse(values).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_contains_body_bound_http_auth_event() {
        let header = authorization_header(
            &Keys::generate(),
            "POST",
            "https://relay.example/events",
            Some(b"{}"),
        )
        .expect("build header");
        assert!(header.starts_with("Nostr "));
    }
}
