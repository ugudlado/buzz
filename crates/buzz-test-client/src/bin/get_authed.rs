//! NIP-98-authed GET against a Buzz relay HTTP endpoint (dev probe).
//! Usage: get_authed <full-url>
//!   env: SIGNER_KEY (hex privkey to sign the NIP-98 auth)

use nostr::Keys;
use std::str::FromStr;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let url = std::env::args().nth(1).expect("url arg");
    let keys = Keys::from_str(&std::env::var("SIGNER_KEY")?)?;
    let authz = buzz_core::http_auth::authorization_header(&keys, "GET", &url, None)
        .map_err(|e| anyhow::anyhow!(e))?;
    let resp = reqwest::Client::new()
        .get(&url)
        .header("Authorization", authz)
        .send()
        .await?;
    let status = resp.status();
    println!("HTTP {status}");
    println!("{}", resp.text().await.unwrap_or_default());
    Ok(())
}
