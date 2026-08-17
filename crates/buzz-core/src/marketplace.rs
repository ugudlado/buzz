//! Shared marketplace listing and pricing types.

use serde::{Deserialize, Serialize};

/// Largest exact integer shared by Rust's JSON values and JavaScript clients.
pub const MAX_MICROUNITS: u64 = 9_007_199_254_740_991;

/// Maximum number of relay identities in a remote-invocation allowlist.
pub const MAX_REMOTE_RELAY_ALLOWLIST: usize = 100;

/// Which external Buzz communities may invoke a listed agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "policy", rename_all = "snake_case", deny_unknown_fields)]
pub enum RemoteInvocationPolicy {
    /// Any authenticated Buzz community relay may submit work.
    AnyCommunity,
    /// Only the listed relay identities may submit work.
    Allowlist {
        /// NIP-11 `self` pubkeys of allowed communities.
        relay_pubkeys: Vec<String>,
    },
}

impl RemoteInvocationPolicy {
    /// Validate and normalize relay identities.
    pub fn normalized(self) -> Result<Self, String> {
        let Self::Allowlist { relay_pubkeys } = self else {
            return Ok(self);
        };
        if relay_pubkeys.is_empty() || relay_pubkeys.len() > MAX_REMOTE_RELAY_ALLOWLIST {
            return Err(format!(
                "remote invocation allowlist must contain 1–{MAX_REMOTE_RELAY_ALLOWLIST} relay pubkeys"
            ));
        }
        let mut normalized = Vec::with_capacity(relay_pubkeys.len());
        for relay_pubkey in relay_pubkeys {
            let relay_pubkey = relay_pubkey.trim().to_lowercase();
            if relay_pubkey.len() != 64
                || !relay_pubkey.bytes().all(|byte| byte.is_ascii_hexdigit())
            {
                return Err(
                    "remote invocation relay pubkeys must be 64 lowercase hex characters".into(),
                );
            }
            if !normalized.contains(&relay_pubkey) {
                normalized.push(relay_pubkey);
            }
        }
        Ok(Self::Allowlist {
            relay_pubkeys: normalized,
        })
    }

    /// Return whether `relay_pubkey` is permitted by this policy.
    pub fn allows(&self, relay_pubkey: &str) -> bool {
        match self {
            Self::AnyCommunity => true,
            Self::Allowlist { relay_pubkeys } => {
                relay_pubkeys.iter().any(|allowed| allowed == relay_pubkey)
            }
        }
    }
}

/// Deployment label shown in the community agent catalog.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentDeployment {
    /// Agent runs on the publisher's local machine.
    Local,
    /// Agent runs on another host.
    Remote,
    /// Agent runs in Kubernetes.
    Kubernetes,
}

/// An agent's duration-based rate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HourlyRate {
    /// Three-letter uppercase ASCII currency code.
    pub currency: String,
    /// Integer micro-units charged per elapsed hour.
    pub microunits_per_hour: u64,
}

impl HourlyRate {
    /// Validate and normalize the rate for storage or publication.
    pub fn normalized(mut self) -> Result<Self, String> {
        self.currency = normalize_currency(&self.currency)?;
        validate_microunits(self.microunits_per_hour)?;
        Ok(self)
    }
}

/// A workflow's optional fixed display price.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FixedPrice {
    /// Three-letter uppercase ASCII currency code.
    pub currency: String,
    /// Fixed display price in integer micro-units.
    pub microunits: u64,
}

impl FixedPrice {
    /// Validate and normalize the fixed price for storage or publication.
    pub fn normalized(mut self) -> Result<Self, String> {
        self.currency = normalize_currency(&self.currency)?;
        validate_microunits(self.microunits)?;
        Ok(self)
    }
}

/// Optional public metadata on a managed-agent projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentMarketplace {
    /// Whether the agent is visible in catalog discovery.
    #[serde(default)]
    pub listed: bool,
    /// Public description, at most 500 characters.
    #[serde(default)]
    pub description: String,
    /// Normalized public capability labels.
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// Descriptive deployment location.
    pub deployment: AgentDeployment,
    /// Optional duration-based rate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pricing: Option<HourlyRate>,
    /// Explicit opt-in policy for work originating in another community.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_invocation: Option<RemoteInvocationPolicy>,
}

impl AgentMarketplace {
    /// Validate and normalize catalog metadata.
    pub fn normalized(mut self) -> Result<Self, String> {
        self.description = self.description.trim().to_string();
        if self.description.chars().count() > 500 {
            return Err("marketplace description must be at most 500 characters".into());
        }
        if self.capabilities.len() > 20 {
            return Err("marketplace capabilities must contain at most 20 entries".into());
        }

        let mut normalized = Vec::with_capacity(self.capabilities.len());
        for capability in self.capabilities {
            let capability = capability.trim().to_lowercase();
            if capability.is_empty() {
                return Err("marketplace capabilities must not be empty".into());
            }
            if capability.chars().count() > 40 {
                return Err("marketplace capabilities must be at most 40 characters".into());
            }
            if !normalized.contains(&capability) {
                normalized.push(capability);
            }
        }
        self.capabilities = normalized;
        self.pricing = self.pricing.map(HourlyRate::normalized).transpose()?;
        self.remote_invocation = self
            .remote_invocation
            .map(RemoteInvocationPolicy::normalized)
            .transpose()?;
        Ok(self)
    }
}

/// Optional listing metadata on a workflow definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowMarketplace {
    /// Whether the workflow is visible in catalog discovery.
    #[serde(default)]
    pub listed: bool,
    /// Public workflow summary, at most 500 characters.
    #[serde(default)]
    pub summary: String,
    /// Optional fixed customer-facing display price.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fixed_price: Option<FixedPrice>,
    /// Source kind:30620 event for an installed marketplace snapshot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_event_id: Option<String>,
}

impl WorkflowMarketplace {
    /// Validate and normalize workflow catalog metadata.
    pub fn normalized(mut self) -> Result<Self, String> {
        self.summary = self.summary.trim().to_string();
        if self.summary.chars().count() > 500 {
            return Err("marketplace workflow summary must be at most 500 characters".into());
        }
        if self.origin_event_id.as_ref().is_some_and(|event_id| {
            event_id.len() != 64 || !event_id.bytes().all(|byte| byte.is_ascii_hexdigit())
        }) {
            return Err("marketplace workflow origin event id must be 64 hex characters".into());
        }
        self.origin_event_id = self.origin_event_id.map(|value| value.to_lowercase());
        self.fixed_price = self.fixed_price.map(FixedPrice::normalized).transpose()?;
        Ok(self)
    }
}

/// An agent's own account of what a completed turn consumed.
///
/// Self-reported and unverified: the agent's harness is the only party that
/// observes model usage, so this travels beside — never replaces — the
/// relay-observed elapsed-time estimate on a receipt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReportedUsage {
    /// Harness that executed the turn (e.g. `goose`, `buzz-agent`).
    pub harness: String,
    /// Effective model id for the turn, when the harness reported one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Turn-level input tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    /// Turn-level output tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
    /// Turn-level cost in integer micro-units of [`Self::currency`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_microunits: Option<u64>,
    /// Three-letter uppercase ASCII currency code. Present iff
    /// [`Self::cost_microunits`] is present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub currency: Option<String>,
}

impl ReportedUsage {
    /// Validate and normalize a self-reported usage estimate.
    pub fn normalized(mut self) -> Result<Self, String> {
        self.harness = self.harness.trim().to_string();
        if self.harness.is_empty() || self.harness.chars().count() > 64 {
            return Err("reported usage harness must contain 1–64 characters".into());
        }
        self.model = match self.model {
            Some(model) => {
                let model = model.trim().to_string();
                if model.chars().count() > 128 {
                    return Err("reported usage model must be at most 128 characters".into());
                }
                (!model.is_empty()).then_some(model)
            }
            None => None,
        };
        if let Some(cost) = self.cost_microunits {
            validate_microunits(cost)?;
        }
        self.currency = match (self.cost_microunits, self.currency) {
            (Some(_), Some(currency)) => Some(normalize_currency(&currency)?),
            (None, None) => None,
            _ => {
                return Err(
                    "reported usage cost_microunits and currency must both be present or absent"
                        .into(),
                );
            }
        };
        Ok(self)
    }
}

/// Calculate floor(rate × duration / one hour) with checked integer arithmetic.
pub fn estimated_microunits(rate: u64, duration_ms: u64) -> Result<u64, String> {
    validate_microunits(rate)?;
    let value = u128::from(rate)
        .checked_mul(u128::from(duration_ms))
        .ok_or_else(|| "estimated value overflow".to_string())?
        / 3_600_000;
    u64::try_from(value).map_err(|_| "estimated value exceeds u64".to_string())
}

fn normalize_currency(currency: &str) -> Result<String, String> {
    let currency = currency.trim();
    if currency.len() != 3 || !currency.bytes().all(|byte| byte.is_ascii_uppercase()) {
        return Err("currency must be exactly three uppercase ASCII letters".into());
    }
    Ok(currency.to_string())
}

fn validate_microunits(value: u64) -> Result<(), String> {
    if value > MAX_MICROUNITS {
        return Err(format!("microunits must not exceed {MAX_MICROUNITS}"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn listing_normalizes_capabilities_and_validates_currency() {
        let listing = AgentMarketplace {
            listed: true,
            description: "  Rust review  ".into(),
            capabilities: vec![" Rust ".into(), "RUST".into(), "Review".into()],
            deployment: AgentDeployment::Remote,
            pricing: Some(HourlyRate {
                currency: "USD".into(),
                microunits_per_hour: 12_000_000,
            }),
            remote_invocation: Some(RemoteInvocationPolicy::Allowlist {
                relay_pubkeys: vec!["A".repeat(64), "a".repeat(64)],
            }),
        }
        .normalized()
        .unwrap();

        assert_eq!(listing.description, "Rust review");
        assert_eq!(listing.capabilities, ["rust", "review"]);
        assert_eq!(
            listing.remote_invocation,
            Some(RemoteInvocationPolicy::Allowlist {
                relay_pubkeys: vec!["a".repeat(64)]
            })
        );
        assert!(HourlyRate {
            currency: "usd".into(),
            microunits_per_hour: 1,
        }
        .normalized()
        .is_err());
    }

    #[test]
    fn listing_rejects_unknown_public_fields() {
        let error = serde_json::from_value::<AgentMarketplace>(serde_json::json!({
            "listed": true,
            "description": "Review",
            "capabilities": ["rust"],
            "deployment": "remote",
            "pricing": { "currency": "USD", "microunits_per_hour": 1, "api_key": "secret" }
        }))
        .expect_err("unknown marketplace fields must not enter a public event");

        assert!(error.to_string().contains("unknown field"));
    }

    fn usage() -> ReportedUsage {
        ReportedUsage {
            harness: "goose".into(),
            model: Some("claude-fable-5".into()),
            input_tokens: Some(1_200),
            output_tokens: Some(340),
            cost_microunits: Some(15_000),
            currency: Some("USD".into()),
        }
    }

    #[test]
    fn reported_usage_normalizes_text_and_requires_a_currency_pair() {
        let normalized = ReportedUsage {
            harness: "  goose  ".into(),
            model: Some("  ".into()),
            ..usage()
        }
        .normalized()
        .expect("normalize usage");
        assert_eq!(normalized.harness, "goose");
        assert_eq!(normalized.model, None);
        assert_eq!(normalized.currency.as_deref(), Some("USD"));

        assert!(ReportedUsage {
            harness: String::new(),
            ..usage()
        }
        .normalized()
        .is_err());
        assert!(ReportedUsage {
            currency: None,
            ..usage()
        }
        .normalized()
        .is_err());
        assert!(ReportedUsage {
            cost_microunits: None,
            ..usage()
        }
        .normalized()
        .is_err());
        assert!(ReportedUsage {
            currency: Some("usd".into()),
            ..usage()
        }
        .normalized()
        .is_err());
        assert!(ReportedUsage {
            cost_microunits: Some(MAX_MICROUNITS + 1),
            ..usage()
        }
        .normalized()
        .is_err());
        assert!(ReportedUsage {
            model: Some("m".repeat(129)),
            ..usage()
        }
        .normalized()
        .is_err());
        assert!(ReportedUsage {
            cost_microunits: None,
            currency: None,
            ..usage()
        }
        .normalized()
        .is_ok());
    }

    #[test]
    fn reported_usage_round_trips_camel_case_and_rejects_unknown_fields() {
        let json = serde_json::to_value(usage()).expect("serialize usage");
        assert_eq!(json["inputTokens"], 1_200);
        assert_eq!(json["costMicrounits"], 15_000);
        assert_eq!(
            serde_json::from_value::<ReportedUsage>(json).expect("round trip"),
            usage()
        );

        let error = serde_json::from_value::<ReportedUsage>(serde_json::json!({
            "harness": "goose",
            "apiKey": "secret"
        }))
        .expect_err("unknown usage fields must be rejected");
        assert!(error.to_string().contains("unknown field"));
    }

    #[test]
    fn reported_usage_omits_absent_optional_fields() {
        let json = serde_json::to_value(ReportedUsage {
            harness: "buzz-agent".into(),
            model: None,
            input_tokens: None,
            output_tokens: None,
            cost_microunits: None,
            currency: None,
        })
        .expect("serialize minimal usage");
        assert_eq!(json, serde_json::json!({ "harness": "buzz-agent" }));
    }

    #[test]
    fn estimate_uses_flooring_and_checked_bounds() {
        assert_eq!(estimated_microunits(12_000_000, 90_000).unwrap(), 300_000);
        assert_eq!(estimated_microunits(1, 3_599_999).unwrap(), 0);
        assert!(estimated_microunits(MAX_MICROUNITS + 1, 1).is_err());
    }
}
