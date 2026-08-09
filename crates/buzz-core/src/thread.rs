//! NIP-10 thread relationship parsing.
//!
//! Shared by any crate that needs to determine an event's thread root/parent
//! or mentioned pubkeys from its `e`/`p` tags — e.g. `buzz-acp` (agent reply
//! threading) and `buzz-relay` (matching an agent's reply to a suspended
//! workflow step).

use nostr::Event;

/// Tag name marking relay-dispatched workflow messages (step prompts and
/// announcements). Prevents recursive workflow triggering and selects
/// workflow-specific handling in the agent harness.
pub const TAG_WORKFLOW: &str = "buzz:workflow";

/// Tag on a harness-posted workflow completion reply naming the prompt event
/// it answers. The relay's resume matcher prefers this over the reply's
/// NIP-10 parent (completions reply to the thread root, not the prompt).
pub const TAG_COMPLETION_OF: &str = "buzz:completion-of";

/// Parsed thread relationship from NIP-10 `e` tags.
#[derive(Debug, Clone, Default)]
pub struct ThreadTags {
    /// Root event ID (hex). Present for all thread replies.
    pub root_event_id: Option<String>,
    /// Parent event ID (hex). For direct replies to root, equals root.
    pub parent_event_id: Option<String>,
    /// Mentioned pubkeys from `p` tags (hex).
    pub mentioned_pubkeys: Vec<String>,
}

/// Parse NIP-10 thread tags from a Nostr event.
///
/// Detection logic (per research doc §4c):
/// - Find an `e` tag with `root` marker → its value is `root_event_id`
/// - Find an `e` tag with `reply` marker → its value is `parent_event_id`
/// - If only `reply` marker found (direct reply to root), root == parent
/// - `p` tags → mentioned pubkeys
///
/// NOTE: Only handles NIP-10 marker-based format (preferred). The deprecated
/// positional format (no markers, `["e", id, relay_url]`) is not supported —
/// Buzz always generates marker-based tags (see relay messages.rs:762-783).
pub fn parse_thread_tags(event: &Event) -> ThreadTags {
    let mut root = None;
    let mut reply = None;
    let mut mentions = Vec::new();

    for tag in event.tags.iter() {
        let parts = tag.as_slice();
        match parts.first().map(|s| s.as_str()) {
            Some("e") if parts.len() >= 4 => {
                let id = &parts[1];
                let marker = &parts[3];
                match marker.as_str() {
                    "root" => root = Some(id.clone()),
                    "reply" => reply = Some(id.clone()),
                    _ => {}
                }
            }
            Some("p") if parts.len() >= 2 => {
                mentions.push(parts[1].clone());
            }
            _ => {}
        }
    }

    // For direct replies to root: single "reply" tag, no "root" tag.
    // In that case, root == parent.
    let (root_event_id, parent_event_id) = match (root, reply) {
        (Some(r), Some(p)) => (Some(r), Some(p)),
        (Some(r), None) => (Some(r.clone()), Some(r)),
        (None, Some(p)) => (Some(p.clone()), Some(p)),
        (None, None) => (None, None),
    };

    ThreadTags {
        root_event_id,
        parent_event_id,
        mentioned_pubkeys: mentions,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind};

    fn make_event(content: &str) -> Event {
        let keys = Keys::generate();
        EventBuilder::new(Kind::Custom(9), content)
            .tags([])
            .sign_with_keys(&keys)
            .unwrap()
    }

    fn make_event_with_tags(content: &str, tags: Vec<Vec<String>>) -> Event {
        let keys = Keys::generate();
        let nostr_tags: Vec<nostr::Tag> = tags
            .into_iter()
            .map(|t| {
                let strs: Vec<&str> = t.iter().map(|s| s.as_str()).collect();
                nostr::Tag::parse(strs).unwrap()
            })
            .collect();
        EventBuilder::new(Kind::Custom(9), content)
            .tags(nostr_tags)
            .sign_with_keys(&keys)
            .unwrap()
    }

    #[test]
    fn test_parse_thread_tags_no_tags() {
        let event = make_event("plain message");
        let tags = parse_thread_tags(&event);
        assert!(tags.root_event_id.is_none());
        assert!(tags.parent_event_id.is_none());
        assert!(tags.mentioned_pubkeys.is_empty());
    }

    #[test]
    fn test_parse_thread_tags_direct_reply() {
        // Direct reply to root: single "reply" tag.
        let event = make_event_with_tags(
            "reply to root",
            vec![vec!["e".into(), "abc123".into(), "".into(), "reply".into()]],
        );
        let tags = parse_thread_tags(&event);
        assert_eq!(tags.root_event_id.as_deref(), Some("abc123"));
        assert_eq!(tags.parent_event_id.as_deref(), Some("abc123"));
    }

    #[test]
    fn test_parse_thread_tags_nested_reply() {
        // Nested reply: root + reply tags.
        let event = make_event_with_tags(
            "nested reply",
            vec![
                vec!["e".into(), "root123".into(), "".into(), "root".into()],
                vec!["e".into(), "parent456".into(), "".into(), "reply".into()],
            ],
        );
        let tags = parse_thread_tags(&event);
        assert_eq!(tags.root_event_id.as_deref(), Some("root123"));
        assert_eq!(tags.parent_event_id.as_deref(), Some("parent456"));
    }

    #[test]
    fn test_parse_thread_tags_with_mentions() {
        let event = make_event_with_tags(
            "hey @alice",
            vec![
                vec!["p".into(), "alice_pubkey".into()],
                vec!["p".into(), "bob_pubkey".into()],
            ],
        );
        let tags = parse_thread_tags(&event);
        assert!(tags.root_event_id.is_none());
        assert_eq!(tags.mentioned_pubkeys, vec!["alice_pubkey", "bob_pubkey"]);
    }

    #[test]
    fn test_parse_thread_tags_root_only() {
        // Only root marker, no reply marker — root == parent.
        let event = make_event_with_tags(
            "reply",
            vec![vec!["e".into(), "root123".into(), "".into(), "root".into()]],
        );
        let tags = parse_thread_tags(&event);
        assert_eq!(tags.root_event_id.as_deref(), Some("root123"));
        assert_eq!(tags.parent_event_id.as_deref(), Some("root123"));
    }
}
