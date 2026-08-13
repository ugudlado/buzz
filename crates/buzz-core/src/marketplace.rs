//! Shared marketplace listing and pricing types.

use serde::{Deserialize, Serialize};

/// Largest exact integer shared by Rust's JSON values and JavaScript clients.
pub const MAX_MICROUNITS: u64 = 9_007_199_254_740_991;

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
}

impl WorkflowMarketplace {
    /// Validate and normalize workflow catalog metadata.
    pub fn normalized(mut self) -> Result<Self, String> {
        self.summary = self.summary.trim().to_string();
        if self.summary.chars().count() > 500 {
            return Err("marketplace workflow summary must be at most 500 characters".into());
        }
        self.fixed_price = self.fixed_price.map(FixedPrice::normalized).transpose()?;
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
        }
        .normalized()
        .unwrap();

        assert_eq!(listing.description, "Rust review");
        assert_eq!(listing.capabilities, ["rust", "review"]);
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

    #[test]
    fn estimate_uses_flooring_and_checked_bounds() {
        assert_eq!(estimated_microunits(12_000_000, 90_000).unwrap(), 300_000);
        assert_eq!(estimated_microunits(1, 3_599_999).unwrap(), 0);
        assert!(estimated_microunits(MAX_MICROUNITS + 1, 1).is_err());
    }
}
