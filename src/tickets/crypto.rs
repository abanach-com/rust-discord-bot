use std::collections::HashMap;
use std::fmt;

use aes_gcm::{
    Aes256Gcm, KeyInit, Nonce,
    aead::{Aead, AeadCore, OsRng, Payload},
};
use anyhow::{Result, anyhow, bail};

pub const TRANSCRIPT_CRYPTO_VERSION: i16 = 1;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EncryptedTranscriptPayload {
    pub ciphertext: Vec<u8>,
    pub nonce: Vec<u8>,
    pub key_id: String,
    pub crypto_version: i16,
}

#[derive(Clone)]
pub struct TranscriptCrypto {
    active_key_id: String,
    keys: HashMap<String, [u8; 32]>,
}

impl fmt::Debug for TranscriptCrypto {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TranscriptCrypto")
            .field("active_key_id", &self.active_key_id)
            .field("available_keys", &self.keys.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl TranscriptCrypto {
    pub fn new(active_key_id: impl Into<String>, key: [u8; 32]) -> Self {
        let active_key_id = active_key_id.into();
        let mut keys = HashMap::new();
        keys.insert(active_key_id.clone(), key);
        Self {
            active_key_id,
            keys,
        }
    }

    pub fn with_keys(
        active_key_id: impl Into<String>,
        keys: HashMap<String, [u8; 32]>,
    ) -> Result<Self> {
        let active_key_id = active_key_id.into();
        if !keys.contains_key(&active_key_id) {
            bail!("active key id '{active_key_id}' not found in provided keyring");
        }
        Ok(Self {
            active_key_id,
            keys,
        })
    }

    pub fn parse_hex_key(raw: &str) -> Result<[u8; 32]> {
        let trimmed = raw.trim();
        if trimmed.len() != 64 {
            bail!(
                "invalid hex key length: expected 64 hex characters (32 bytes), got {}",
                trimmed.len()
            );
        }
        let mut key = [0u8; 32];
        hex::decode_to_slice(trimmed, &mut key)
            .map_err(|e| anyhow!("failed to decode hex key: {e}"))?;
        Ok(key)
    }

    pub fn from_hex_key(active_key_id: impl Into<String>, hex_key: &str) -> Result<Self> {
        let key = Self::parse_hex_key(hex_key)?;
        Ok(Self::new(active_key_id, key))
    }

    pub fn from_keyring_spec(active_key_id: impl Into<String>, spec: &str) -> Result<Self> {
        let trimmed = spec.trim();
        if trimmed.starts_with('{') {
            let parsed: HashMap<String, String> = serde_json::from_str(trimmed)
                .map_err(|e| anyhow!("failed to parse JSON keyring spec: {e}"))?;
            let mut keys = HashMap::new();
            for (id, hex_val) in parsed {
                let key = Self::parse_hex_key(&hex_val)?;
                keys.insert(id, key);
            }
            return Self::with_keys(active_key_id, keys);
        }

        let mut keys = HashMap::new();
        for entry in trimmed.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            if let Some((id, hex_val)) = entry.split_once(':') {
                let key = Self::parse_hex_key(hex_val)?;
                keys.insert(id.trim().to_string(), key);
            } else if let Some((id, hex_val)) = entry.split_once('=') {
                let key = Self::parse_hex_key(hex_val)?;
                keys.insert(id.trim().to_string(), key);
            } else {
                let active = active_key_id.into();
                let key = Self::parse_hex_key(entry)?;
                keys.insert(active.clone(), key);
                return Self::with_keys(active, keys);
            }
        }

        Self::with_keys(active_key_id, keys)
    }

    #[must_use]
    pub fn active_key_id(&self) -> &str {
        &self.active_key_id
    }

    #[must_use]
    pub fn aad(guild_id: &str, ticket_id: &str) -> String {
        format!("transcript:v1:guild={guild_id}:ticket={ticket_id}")
    }

    pub fn encrypt(
        &self,
        guild_id: &str,
        ticket_id: &str,
        cleartext: &str,
    ) -> Result<EncryptedTranscriptPayload> {
        let key = self
            .keys
            .get(&self.active_key_id)
            .ok_or_else(|| anyhow!("active key '{}' not found in keyring", self.active_key_id))?;
        let cipher =
            Aes256Gcm::new_from_slice(key).map_err(|e| anyhow!("invalid transcript key: {e}"))?;
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let aad = Self::aad(guild_id, ticket_id);
        let payload = Payload {
            msg: cleartext.as_bytes(),
            aad: aad.as_bytes(),
        };
        let ciphertext = cipher
            .encrypt(&nonce, payload)
            .map_err(|_| anyhow!("transcript encryption failed"))?;

        Ok(EncryptedTranscriptPayload {
            ciphertext,
            nonce: nonce.to_vec(),
            key_id: self.active_key_id.clone(),
            crypto_version: TRANSCRIPT_CRYPTO_VERSION,
        })
    }

    pub fn decrypt(
        &self,
        key_id: &str,
        guild_id: &str,
        ticket_id: &str,
        nonce_bytes: &[u8],
        ciphertext: &[u8],
    ) -> Result<String> {
        let key = self
            .keys
            .get(key_id)
            .ok_or_else(|| anyhow!("key '{key_id}' not found in keyring"))?;
        let cipher =
            Aes256Gcm::new_from_slice(key).map_err(|e| anyhow!("invalid transcript key: {e}"))?;
        if nonce_bytes.len() != 12 {
            bail!(
                "invalid nonce length: expected 12 bytes, got {}",
                nonce_bytes.len()
            );
        }
        let nonce = Nonce::from_slice(nonce_bytes);
        let aad = Self::aad(guild_id, ticket_id);
        let payload = Payload {
            msg: ciphertext,
            aad: aad.as_bytes(),
        };
        let decrypted_bytes = cipher.decrypt(nonce, payload).map_err(|_| {
            anyhow!(
                "transcript decryption failed: authenticated data mismatch or invalid ciphertext"
            )
        })?;

        String::from_utf8(decrypted_bytes)
            .map_err(|e| anyhow!("decrypted transcript is not valid utf-8: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_transcript_encryption_roundtrip_and_nonce_uniqueness() {
        let key = [0x42u8; 32];
        let crypto = TranscriptCrypto::new("v1", key);
        let guild_id = "100200300";
        let ticket_id = "ticket-uuid-abc";
        let cleartext = "Operator: Hello user!\nUser: I need help with server roles.";

        let enc1 = crypto.encrypt(guild_id, ticket_id, cleartext).unwrap();
        let enc2 = crypto.encrypt(guild_id, ticket_id, cleartext).unwrap();

        assert_eq!(enc1.key_id, "v1");
        assert_eq!(enc1.crypto_version, TRANSCRIPT_CRYPTO_VERSION);
        assert_ne!(
            enc1.nonce, enc2.nonce,
            "Subsequent encryptions must have unique nonces"
        );
        assert_ne!(
            enc1.ciphertext, enc2.ciphertext,
            "Ciphertexts must differ due to unique nonces"
        );

        let dec1 = crypto
            .decrypt(
                &enc1.key_id,
                guild_id,
                ticket_id,
                &enc1.nonce,
                &enc1.ciphertext,
            )
            .unwrap();
        assert_eq!(dec1, cleartext);

        let dec2 = crypto
            .decrypt(
                &enc2.key_id,
                guild_id,
                ticket_id,
                &enc2.nonce,
                &enc2.ciphertext,
            )
            .unwrap();
        assert_eq!(dec2, cleartext);
    }

    #[test]
    fn test_transcript_tamper_and_aad_mismatch_fails_decryption() {
        let key = [0x55u8; 32];
        let crypto = TranscriptCrypto::new("key_primary", key);
        let guild_id = "guild_alpha";
        let ticket_id = "ticket_beta";
        let cleartext = "Sensitive customer data";

        let enc = crypto.encrypt(guild_id, ticket_id, cleartext).unwrap();

        // 1. Wrong guild_id in AAD
        let err_guild = crypto.decrypt(
            &enc.key_id,
            "guild_other",
            ticket_id,
            &enc.nonce,
            &enc.ciphertext,
        );
        assert!(
            err_guild.is_err(),
            "Decryption must fail with mismatched guild AAD"
        );

        // 2. Wrong ticket_id in AAD
        let err_ticket = crypto.decrypt(
            &enc.key_id,
            guild_id,
            "ticket_other",
            &enc.nonce,
            &enc.ciphertext,
        );
        assert!(
            err_ticket.is_err(),
            "Decryption must fail with mismatched ticket AAD"
        );

        // 3. Corrupted ciphertext
        let mut corrupted_ciphertext = enc.ciphertext.clone();
        corrupted_ciphertext[0] ^= 0xFF;
        let err_tamper = crypto.decrypt(
            &enc.key_id,
            guild_id,
            ticket_id,
            &enc.nonce,
            &corrupted_ciphertext,
        );
        assert!(
            err_tamper.is_err(),
            "Decryption must fail with tampered ciphertext"
        );
    }

    #[test]
    fn test_transcript_keyring_rotation() {
        let key_v1 = [0x11u8; 32];
        let key_v2 = [0x22u8; 32];

        let mut keyring = HashMap::new();
        keyring.insert("v1".to_string(), key_v1);
        keyring.insert("v2".to_string(), key_v2);

        // Active key is v2
        let crypto = TranscriptCrypto::with_keys("v2", keyring).unwrap();
        assert_eq!(crypto.active_key_id(), "v2");

        let guild_id = "guild_1";
        let ticket_id = "ticket_1";

        // Encrypt with v2
        let enc_v2 = crypto
            .encrypt(guild_id, ticket_id, "Message encrypted under v2")
            .unwrap();
        assert_eq!(enc_v2.key_id, "v2");

        // Can decrypt record encrypted under v1 using v1 key in keyring
        let crypto_v1_legacy = TranscriptCrypto::new("v1", key_v1);
        let enc_v1 = crypto_v1_legacy
            .encrypt(guild_id, ticket_id, "Legacy message encrypted under v1")
            .unwrap();
        let dec_v1 = crypto
            .decrypt(
                &enc_v1.key_id,
                guild_id,
                ticket_id,
                &enc_v1.nonce,
                &enc_v1.ciphertext,
            )
            .unwrap();
        assert_eq!(dec_v1, "Legacy message encrypted under v1");

        // Unknown key fails
        let err_unknown = crypto.decrypt(
            "v999",
            guild_id,
            ticket_id,
            &enc_v1.nonce,
            &enc_v1.ciphertext,
        );
        assert!(err_unknown.is_err());
    }

    #[test]
    fn test_parse_hex_and_keyring_specs() {
        let raw_hex = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let parsed = TranscriptCrypto::parse_hex_key(raw_hex).unwrap();
        assert_eq!(parsed[0], 0x01);
        assert_eq!(parsed[31], 0xef);

        // from_hex_key
        let crypto = TranscriptCrypto::from_hex_key("v1", raw_hex).unwrap();
        assert_eq!(crypto.active_key_id(), "v1");

        // comma spec
        let raw_hex2 = "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210";
        let spec = format!("v1:{raw_hex},v2:{raw_hex2}");
        let crypto_spec = TranscriptCrypto::from_keyring_spec("v2", &spec).unwrap();
        assert_eq!(crypto_spec.active_key_id(), "v2");

        // debug does not leak secret bytes
        let debug_str = format!("{crypto_spec:?}");
        assert!(!debug_str.contains(raw_hex));
        assert!(!debug_str.contains(raw_hex2));
        assert!(debug_str.contains("available_keys"));
    }
}
