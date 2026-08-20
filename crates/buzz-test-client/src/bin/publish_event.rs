//! Publish an arbitrary signed event to a relay — dev/test seeding helper.
//!
//! Usage: publish_event <kind> <tags-json> [content]
//!   env: BUZZ_RELAY_URL (ws/wss), SEED_PRIVATE_KEY (hex)
//!   Content may also be piped on stdin when the arg is omitted or "-".
//!   tags-json example: '[["d","<pubkey>"],["t","x"]]'

use buzz_test_client::BuzzTestClient;
use nostr::{EventBuilder, Keys, Kind, Tag};
use std::io::Read;
use std::str::FromStr;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let args: Vec<String> = std::env::args().collect();
    // `publish_event --pubkey` prints the hex pubkey for SEED_PRIVATE_KEY.
    if args.get(1).map(String::as_str) == Some("--pubkey") {
        let key_hex = std::env::var("SEED_PRIVATE_KEY").expect("SEED_PRIVATE_KEY required");
        let keys = Keys::from_str(&key_hex)?;
        println!("{}", keys.public_key().to_hex());
        return Ok(());
    }
    if args.len() < 3 {
        eprintln!("Usage: publish_event <kind> <tags-json> [content|-]");
        eprintln!("       publish_event --pubkey");
        std::process::exit(1);
    }
    let kind: u16 = args[1].parse()?;
    let raw_tags: Vec<Vec<String>> = serde_json::from_str(&args[2])?;
    let content = match args.get(3).map(String::as_str) {
        Some("-") | None => {
            let mut buf = String::new();
            std::io::stdin().read_to_string(&mut buf)?;
            buf
        }
        Some(arg) => arg.to_string(),
    };

    let url = std::env::var("BUZZ_RELAY_URL").unwrap_or_else(|_| "ws://localhost:3000".into());
    let key_hex = std::env::var("SEED_PRIVATE_KEY").expect("SEED_PRIVATE_KEY required");
    let keys = Keys::from_str(&key_hex)?;

    let mut tags = Vec::new();
    for t in raw_tags {
        tags.push(Tag::parse(t)?);
    }
    let event = EventBuilder::new(Kind::Custom(kind), content)
        .tags(tags)
        .sign_with_keys(&keys)?;

    let mut client = BuzzTestClient::connect(&url, &keys).await?;
    let ok = client.send_event(event).await?;
    if ok.accepted {
        println!("{}", ok.event_id);
    } else {
        eprintln!("rejected: {}", ok.message);
        std::process::exit(2);
    }
    Ok(())
}
