use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComponentPayload {
    pub guild_id: String,
    pub user_id: String,
    pub action: String,
    pub payload: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ComponentError {
    #[error("invalid component id format")]
    InvalidFormat,
    #[error("component signature mismatch")]
    InvalidSignature,
    #[error("component interaction expired")]
    Expired,
    #[error("component was issued for a different guild")]
    GuildMismatch,
    #[error("user is not authorized to interact with this component")]
    UserMismatch,
}

pub fn compute_signature(secret: &str, data: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(secret.as_bytes());
    hasher.update(b":");
    hasher.update(data.as_bytes());
    hex::encode(&hasher.finalize()[..8])
}

pub fn encode_component_id(
    secret: &str,
    guild_id: &str,
    user_id: &str,
    action: &str,
    payload: &str,
    ttl_seconds: i64,
) -> String {
    let expires_at = Utc::now().timestamp() + ttl_seconds;
    let body = format!("{expires_at}:{guild_id}:{user_id}:{action}:{payload}");
    let sig = compute_signature(secret, &body);
    format!("v1:{sig}:{body}")
}

pub fn decode_and_validate_component_id(
    secret: &str,
    custom_id: &str,
    caller_user_id: &str,
    current_guild_id: &str,
) -> Result<ComponentPayload, ComponentError> {
    let parts: Vec<&str> = custom_id.splitn(7, ':').collect();
    if parts.len() < 7 || parts[0] != "v1" {
        return Err(ComponentError::InvalidFormat);
    }

    let sig = parts[1];
    let expires_at_str = parts[2];
    let guild_id = parts[3];
    let user_id = parts[4];
    let action = parts[5];
    let payload = parts[6];

    let body = format!("{expires_at_str}:{guild_id}:{user_id}:{action}:{payload}");
    let expected_sig = compute_signature(secret, &body);
    if sig != expected_sig {
        return Err(ComponentError::InvalidSignature);
    }

    let expires_at_ts = expires_at_str
        .parse::<i64>()
        .map_err(|_| ComponentError::InvalidFormat)?;
    if Utc::now().timestamp() > expires_at_ts {
        return Err(ComponentError::Expired);
    }

    if guild_id != current_guild_id {
        return Err(ComponentError::GuildMismatch);
    }

    if user_id != "*" && user_id != caller_user_id {
        return Err(ComponentError::UserMismatch);
    }

    let expires_at = DateTime::from_timestamp(expires_at_ts, 0).unwrap_or_else(Utc::now);

    Ok(ComponentPayload {
        guild_id: guild_id.to_owned(),
        user_id: user_id.to_owned(),
        action: action.to_owned(),
        payload: payload.to_owned(),
        expires_at,
    })
}

pub fn verify_module_manifest_checksum(xml_content: &str, expected_hash: &str) -> bool {
    let mut hasher = Sha256::new();
    hasher.update(xml_content.as_bytes());
    let hash = hex::encode(hasher.finalize());
    hash.eq_ignore_ascii_case(expected_hash)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_component_id_roundtrip_and_validation() {
        let secret = "super_secret_test_key";
        let guild_id = "100000000000000001";
        let user_id = "200000000000000002";
        let action = "confirm_ticket";
        let payload = "ticket_42";

        let encoded = encode_component_id(secret, guild_id, user_id, action, payload, 60);
        let res = decode_and_validate_component_id(secret, &encoded, user_id, guild_id).unwrap();

        assert_eq!(res.guild_id, guild_id);
        assert_eq!(res.user_id, user_id);
        assert_eq!(res.action, action);
        assert_eq!(res.payload, payload);

        // Caller mismatch
        let mismatch_caller =
            decode_and_validate_component_id(secret, &encoded, "333333333", guild_id);
        assert_eq!(mismatch_caller, Err(ComponentError::UserMismatch));

        // Guild mismatch
        let mismatch_guild =
            decode_and_validate_component_id(secret, &encoded, user_id, "999999999");
        assert_eq!(mismatch_guild, Err(ComponentError::GuildMismatch));

        // Signature forgery
        let mut tampered = encoded.clone();
        tampered.push_str("_tampered");
        let forged = decode_and_validate_component_id(secret, &tampered, user_id, guild_id);
        assert_eq!(forged, Err(ComponentError::InvalidSignature));

        // Expired
        let expired_id = encode_component_id(secret, guild_id, user_id, action, payload, -10);
        let exp_res = decode_and_validate_component_id(secret, &expired_id, user_id, guild_id);
        assert_eq!(exp_res, Err(ComponentError::Expired));
    }
}
