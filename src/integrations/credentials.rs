use std::collections::HashMap;
use std::fmt;

use aes_gcm::{
    Aes256Gcm, KeyInit, Nonce,
    aead::{Aead, AeadCore, OsRng, Payload},
};
use anyhow::{Result, anyhow, bail};

pub const CREDENTIAL_CRYPTO_VERSION: &str = "v1";
pub const CREDENTIAL_ENVELOPE_PREFIX: &str = "enc:v1:";

/// Manages application-layer encryption and decryption for integration credentials at rest (AES-256-GCM).
///
/// Features:
/// - Distinct domain keyring, isolated from ticket transcripts or other subsystem keys.
/// - Random 96-bit nonce from `OsRng` generated on every encryption call.
/// - Strict Additional Authenticated Data (AAD) binding tenant isolation:
///   `integration:v1:guild={guild_id}:entity={entity_type}:id={record_id}:field={field_name}`
/// - Envelope format: `enc:v1:<key_id>:<nonce_hex>:<ciphertext_hex>`
/// - Fail-closed on decryption failures (tampering, wrong guild, wrong record, missing key).
/// - Transparent backwards-compatible reading of unencrypted legacy values during migration phases.
#[derive(Clone)]
pub struct CredentialCrypto {
    active_key_id: String,
    keys: HashMap<String, [u8; 32]>,
}

impl fmt::Debug for CredentialCrypto {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CredentialCrypto")
            .field("active_key_id", &self.active_key_id)
            .field("available_keys", &self.keys.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl CredentialCrypto {
    /// Create a single-key crypto context.
    #[must_use]
    pub fn new(active_key_id: impl Into<String>, key: [u8; 32]) -> Self {
        let active_key_id = active_key_id.into();
        let mut keys = HashMap::new();
        keys.insert(active_key_id.clone(), key);
        Self {
            active_key_id,
            keys,
        }
    }

    /// Create a keyring-based crypto context with an active key ID.
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

    /// Generate an ephemeral random key (useful for tests and non-persistent dev environments).
    #[must_use]
    pub fn generate_ephemeral() -> Self {
        let gen_key = Aes256Gcm::generate_key(&mut OsRng);
        let mut key = [0u8; 32];
        key.copy_from_slice(&gen_key);
        Self::new("ephemeral", key)
    }

    /// Parse a 64-character hex string into a 32-byte key.
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

    /// Create from a single hex-encoded 32-byte key.
    pub fn from_hex_key(active_key_id: impl Into<String>, hex_key: &str) -> Result<Self> {
        let key = Self::parse_hex_key(hex_key)?;
        Ok(Self::new(active_key_id, key))
    }

    /// Create from a keyring specification (JSON map or comma-separated `id:hex` / `id=hex`).
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

    /// Check if a stored string is wrapped in an encrypted envelope.
    #[must_use]
    pub fn is_encrypted(val: &str) -> bool {
        val.starts_with(CREDENTIAL_ENVELOPE_PREFIX)
    }

    /// Construct canonical Additional Authenticated Data (AAD) for credential isolation.
    #[must_use]
    pub fn aad(guild_id: &str, entity_type: &str, record_id: &str, field_name: &str) -> String {
        format!(
            "integration:v1:guild={guild_id}:entity={entity_type}:id={record_id}:field={field_name}"
        )
    }

    /// Encrypt a credential field using the active key and canonical AAD.
    ///
    /// Returns the envelope formatted as `enc:v1:<key_id>:<nonce_hex>:<ciphertext_hex>`.
    pub fn encrypt_field(
        &self,
        guild_id: &str,
        entity_type: &str,
        record_id: &str,
        field_name: &str,
        cleartext: &str,
    ) -> Result<String> {
        let key = self.keys.get(&self.active_key_id).ok_or_else(|| {
            anyhow!(
                "active integration credential key '{}' not found in keyring",
                self.active_key_id
            )
        })?;
        let cipher = Aes256Gcm::new_from_slice(key)
            .map_err(|e| anyhow!("invalid integration credential key: {e}"))?;
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let aad = Self::aad(guild_id, entity_type, record_id, field_name);
        let payload = Payload {
            msg: cleartext.as_bytes(),
            aad: aad.as_bytes(),
        };
        let ciphertext = cipher
            .encrypt(&nonce, payload)
            .map_err(|_| anyhow!("integration credential encryption failed"))?;

        let nonce_hex = hex::encode(nonce);
        let ciphertext_hex = hex::encode(ciphertext);

        Ok(format!(
            "{}{}:{}:{}",
            CREDENTIAL_ENVELOPE_PREFIX, self.active_key_id, nonce_hex, ciphertext_hex
        ))
    }

    /// Decrypt a credential field from an envelope string or transparently pass legacy plaintext.
    ///
    /// Fail-closed if ciphertext or authenticated data (guild, entity, id, field) has been tampered with.
    pub fn decrypt_field(
        &self,
        guild_id: &str,
        entity_type: &str,
        record_id: &str,
        field_name: &str,
        stored_value: &str,
    ) -> Result<String> {
        if !Self::is_encrypted(stored_value) {
            tracing::warn!(
                guild_id = %guild_id,
                entity_type = %entity_type,
                record_id = %record_id,
                field_name = %field_name,
                "Unencrypted legacy integration credential detected; migration/backfill recommended"
            );
            return Ok(stored_value.to_string());
        }

        // Parse: enc:v1:<key_id>:<nonce_hex>:<ciphertext_hex>
        let payload = &stored_value[CREDENTIAL_ENVELOPE_PREFIX.len()..];
        let parts: Vec<&str> = payload.split(':').collect();
        if parts.len() != 3 {
            bail!(
                "malformed integration credential envelope: expected <key_id>:<nonce_hex>:<ciphertext_hex>"
            );
        }

        let key_id = parts[0];
        let nonce_hex = parts[1];
        let ciphertext_hex = parts[2];

        let key = self
            .keys
            .get(key_id)
            .ok_or_else(|| anyhow!("integration credential key '{key_id}' not found in keyring"))?;
        let cipher = Aes256Gcm::new_from_slice(key)
            .map_err(|e| anyhow!("invalid integration credential key: {e}"))?;

        let nonce_bytes =
            hex::decode(nonce_hex).map_err(|e| anyhow!("invalid nonce hex encoding: {e}"))?;
        if nonce_bytes.len() != 12 {
            bail!(
                "invalid nonce length: expected 12 bytes, got {}",
                nonce_bytes.len()
            );
        }
        let nonce = Nonce::from_slice(&nonce_bytes);

        let ciphertext_bytes = hex::decode(ciphertext_hex)
            .map_err(|e| anyhow!("invalid ciphertext hex encoding: {e}"))?;

        let aad = Self::aad(guild_id, entity_type, record_id, field_name);
        let gcm_payload = Payload {
            msg: &ciphertext_bytes,
            aad: aad.as_bytes(),
        };

        let decrypted_bytes = cipher.decrypt(nonce, gcm_payload).map_err(|_| {
            anyhow!("integration credential decryption failed: authenticated data mismatch, tampering, or invalid ciphertext")
        })?;

        String::from_utf8(decrypted_bytes)
            .map_err(|e| anyhow!("decrypted credential is not valid utf-8: {e}"))
    }

    /// Helper for optional string encryption.
    pub fn encrypt_opt(
        &self,
        guild_id: &str,
        entity_type: &str,
        record_id: &str,
        field_name: &str,
        val: Option<&str>,
    ) -> Result<Option<String>> {
        match val {
            Some(v) => Ok(Some(self.encrypt_field(
                guild_id,
                entity_type,
                record_id,
                field_name,
                v,
            )?)),
            None => Ok(None),
        }
    }

    /// Helper for optional string decryption.
    pub fn decrypt_opt(
        &self,
        guild_id: &str,
        entity_type: &str,
        record_id: &str,
        field_name: &str,
        val: Option<&str>,
    ) -> Result<Option<String>> {
        match val {
            Some(v) => Ok(Some(self.decrypt_field(
                guild_id,
                entity_type,
                record_id,
                field_name,
                v,
            )?)),
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_credential_encryption_roundtrip_and_nonce_uniqueness() {
        let key = [0x55u8; 32];
        let crypto = CredentialCrypto::new("v1", key);
        let guild_id = "123456789";
        let entity = "telegram_bridge";
        let record_id = "bridge-uuid-001";
        let field = "secret_token";
        let cleartext = "secret_token_value_abc_xyz_123";

        let enc1 = crypto
            .encrypt_field(guild_id, entity, record_id, field, cleartext)
            .unwrap();
        let enc2 = crypto
            .encrypt_field(guild_id, entity, record_id, field, cleartext)
            .unwrap();

        assert!(enc1.starts_with(CREDENTIAL_ENVELOPE_PREFIX));
        assert!(enc2.starts_with(CREDENTIAL_ENVELOPE_PREFIX));
        assert_ne!(enc1, enc2, "Each encryption must use a unique random nonce");

        let dec1 = crypto
            .decrypt_field(guild_id, entity, record_id, field, &enc1)
            .unwrap();
        let dec2 = crypto
            .decrypt_field(guild_id, entity, record_id, field, &enc2)
            .unwrap();

        assert_eq!(dec1, cleartext);
        assert_eq!(dec2, cleartext);
    }

    #[test]
    fn test_credential_tampering_and_tenant_isolation_fail_closed() {
        let key = [0x77u8; 32];
        let crypto = CredentialCrypto::new("v1", key);
        let guild_id = "1000";
        let entity = "telegram_bridge";
        let record_id = "bridge-uuid-1000";
        let field = "secret_token";
        let cleartext = "super-secret-integration-token";

        let enc = crypto
            .encrypt_field(guild_id, entity, record_id, field, cleartext)
            .unwrap();

        // 1. Cross-tenant tampering: wrong guild_id
        let err_guild = crypto.decrypt_field("9999", entity, record_id, field, &enc);
        assert!(
            err_guild.is_err(),
            "Decryption with different guild_id must fail"
        );

        // 2. Entity confusion: wrong entity_type
        let err_entity =
            crypto.decrypt_field(guild_id, "integration_connection", record_id, field, &enc);
        assert!(
            err_entity.is_err(),
            "Decryption with different entity_type must fail"
        );

        // 3. Cross-record tampering: wrong record_id
        let err_record = crypto.decrypt_field(guild_id, entity, "bridge-uuid-2000", field, &enc);
        assert!(
            err_record.is_err(),
            "Decryption with different record_id must fail"
        );

        // 4. Cross-field tampering: access_token vs refresh_token
        let err_field = crypto.decrypt_field(guild_id, entity, record_id, "access_token", &enc);
        assert!(
            err_field.is_err(),
            "Decryption with different field name must fail"
        );

        // 5. Corrupted ciphertext
        let parts: Vec<&str> = enc[CREDENTIAL_ENVELOPE_PREFIX.len()..].split(':').collect();
        let corrupted_ciphertext = format!(
            "{}{}:{}:{}",
            CREDENTIAL_ENVELOPE_PREFIX,
            parts[0],
            parts[1],
            &parts[2][..parts[2].len() - 4]
        );
        let err_corrupt =
            crypto.decrypt_field(guild_id, entity, record_id, field, &corrupted_ciphertext);
        assert!(
            err_corrupt.is_err(),
            "Decryption with corrupted ciphertext must fail"
        );
    }

    #[test]
    fn test_key_rotation_and_keyring() {
        let key_v1 = [0x11u8; 32];
        let key_v2 = [0x22u8; 32];
        let mut keys = HashMap::new();
        keys.insert("v1".to_string(), key_v1);
        keys.insert("v2".to_string(), key_v2);

        // Active key is v2, but v1 is still in keyring
        let crypto = CredentialCrypto::with_keys("v2", keys).unwrap();

        // Encrypt with older crypto having v1 active
        let crypto_v1 = CredentialCrypto::new("v1", key_v1);
        let enc_old = crypto_v1
            .encrypt_field("123", "integration", "rec-1", "token", "token-val")
            .unwrap();
        assert!(enc_old.contains(":v1:"));

        // New crypto (active v2) can decrypt legacy v1 record
        let dec = crypto
            .decrypt_field("123", "integration", "rec-1", "token", &enc_old)
            .unwrap();
        assert_eq!(dec, "token-val");

        // New encryptions use v2
        let enc_new = crypto
            .encrypt_field("123", "integration", "rec-1", "token", "new-token-val")
            .unwrap();
        assert!(enc_new.contains(":v2:"));
    }

    #[test]
    fn test_legacy_plaintext_passthrough_with_warning() {
        let crypto = CredentialCrypto::generate_ephemeral();
        let legacy_val = "plaintext-secret-token-from-db-backup";
        let dec = crypto
            .decrypt_field(
                "123",
                "telegram_bridge",
                "bridge-1",
                "secret_token",
                legacy_val,
            )
            .unwrap();
        assert_eq!(dec, legacy_val);
    }
}
