use nostr::nips::nip19::FromBech32;

pub struct Identity {
    pubkey: String,
}

impl Identity {
    pub fn from_nsec(nsec: &str) -> Result<Self, String> {
        let secret = nostr::SecretKey::from_bech32(nsec.trim())
            .map_err(|_| "private_key_nsec is not a decodable nsec1 key".to_string())?;
        Ok(Self {
            pubkey: nostr::Keys::new(secret).public_key().to_hex(),
        })
    }

    pub fn service_name(&self) -> String {
        format!("buzz-agent-{}.service", &self.pubkey[..12])
    }

    pub fn pubkey(&self) -> &str {
        &self.pubkey
    }

    pub fn state_name(&self) -> String {
        format!("buzz-agent-{}", &self.pubkey[..12])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::nips::nip19::ToBech32;

    #[test]
    fn service_name_is_derived_from_the_nsec() {
        let keys = nostr::Keys::generate();
        let nsec = keys.secret_key().to_bech32().unwrap();
        let id = Identity::from_nsec(&nsec).unwrap();
        assert_eq!(
            id.service_name(),
            format!("buzz-agent-{}.service", &keys.public_key().to_hex()[..12])
        );
        assert!(Identity::from_nsec("not-a-key").is_err());
    }
}
