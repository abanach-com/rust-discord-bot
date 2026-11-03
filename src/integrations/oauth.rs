use chrono::{DateTime, Utc};

use dashmap::DashMap;

use sha2::{Digest, Sha256};

use std::sync::Arc;

use tokio::sync::Mutex;

use uuid::Uuid;



use super::IntegrationError;



pub fn base64url_encode(input: &[u8]) -> String {

    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

    let mut out = String::new();

    let mut i = 0;

    while i < input.len() {

        let b0 = input[i] as u32;

        let b1 = if i + 1 < input.len() { input[i + 1] as u32 } else { 0 };

        let b2 = if i + 2 < input.len() { input[i + 2] as u32 } else { 0 };

        let triple = (b0 << 16) | (b1 << 8) | b2;

        out.push(TABLE[((triple >> 18) & 63) as usize] as char);

        out.push(TABLE[((triple >> 12) & 63) as usize] as char);

        if i + 1 < input.len() {

            out.push(TABLE[((triple >> 6) & 63) as usize] as char);

        }

        if i + 2 < input.len() {

            out.push(TABLE[(triple & 63) as usize] as char);

        }

        i += 3;

    }

    out

}



/// Generates a PKCE code_verifier and code_challenge (RFC 7636, S256 with BASE64URL no padding).

pub fn generate_pkce_pair() -> (String, String) {

    let verifier = format!("{}-{}", Uuid::new_v4(), Uuid::new_v4()).replace('-', "");

    let mut hasher = Sha256::new();

    hasher.update(verifier.as_bytes());

    let hash = hasher.finalize();

    let challenge = base64url_encode(&hash);

    (verifier, challenge)

}



#[derive(Debug, Clone)]

pub struct PendingOAuthSession {

    pub state: String,

    pub nonce: String,

    pub guild_id: String,

    pub user_id: String,

    pub provider: String,

    pub code_verifier: String,

    pub expires_at: DateTime<Utc>,

}



#[derive(Default)]

pub struct OAuthManager {

    sessions: DashMap<String, PendingOAuthSession>,

    refresh_locks: DashMap<String, Arc<Mutex<()>>>,

}



impl OAuthManager {

    pub fn new() -> Self {

        Self {

            sessions: DashMap::new(),

            refresh_locks: DashMap::new(),

        }

    }



    pub fn create_session(

        &self,

        guild_id: &str,

        user_id: &str,

        provider: &str,

        ttl_seconds: i64,

    ) -> PendingOAuthSession {

        let (verifier, _) = generate_pkce_pair();

        let state = format!("{}_{}", provider, Uuid::new_v4().simple());

        let nonce = Uuid::new_v4().to_string();

        let expires_at = Utc::now() + chrono::Duration::seconds(ttl_seconds);



        let session = PendingOAuthSession {

            state: state.clone(),

            nonce,

            guild_id: guild_id.to_string(),

            user_id: user_id.to_string(),

            provider: provider.to_string(),

            code_verifier: verifier,

            expires_at,

        };



        self.sessions.insert(state.clone(), session.clone());

        session

    }



    pub fn consume_session(

        &self,

        state: &str,

        expected_guild: &str,

    ) -> Result<PendingOAuthSession, IntegrationError> {

        let (_, session) = self.sessions.remove(state).ok_or_else(|| {

            IntegrationError::AuthenticationFailed("Invalid or expired OAuth state".to_string())

        })?;



        if Utc::now() > session.expires_at {

            return Err(IntegrationError::AuthenticationFailed(

                "OAuth session expired".to_string(),

            ));

        }



        if session.guild_id != expected_guild {

            return Err(IntegrationError::TenantViolation(

                "State session belongs to another guild".to_string(),

            ));

        }



        Ok(session)

    }



    /// Single-flight lock coordinator for per-connection token refreshes

    pub async fn get_refresh_lock(&self, connection_id: &str) -> Arc<Mutex<()>> {

        self.refresh_locks

            .entry(connection_id.to_string())

            .or_insert_with(|| Arc::new(Mutex::new(())))

            .clone()

    }

}



// ==========================================

// Steam OpenID Assertion Validator

// ==========================================



pub fn extract_steam_id_from_claimed_id(claimed_id: &str) -> Option<String> {

    // Pattern: https://steamcommunity.com/openid/id/76561198000000000

    let prefix = "https://steamcommunity.com/openid/id/";

    if let Some(id_str) = claimed_id.strip_prefix(prefix)

        && id_str.chars().all(|c| c.is_ascii_digit())

        && id_str.len() >= 17

    {

        return Some(id_str.to_string());

    }

    None

}



pub fn verify_steam_openid_assertion(

    params: &std::collections::HashMap<String, String>,

) -> Result<String, IntegrationError> {

    let mode = params.get("openid.mode").map(|s| s.as_str());

    if mode != Some("id_res") {

        return Err(IntegrationError::AuthenticationFailed(

            "Steam OpenID mode is not id_res".to_string(),

        ));

    }



    let claimed_id = params.get("openid.claimed_id").ok_or_else(|| {

        IntegrationError::AuthenticationFailed("Missing openid.claimed_id".to_string())

    })?;



    let steam_id = extract_steam_id_from_claimed_id(claimed_id).ok_or_else(|| {

        IntegrationError::AuthenticationFailed("Invalid Steam OpenID format".to_string())

    })?;



    Ok(steam_id)

}



#[cfg(test)]

mod tests {

    use super::*;



    #[test]

    fn test_pkce_rfc7636_appendix_b_vector() {

        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";

        let mut hasher = Sha256::new();

        hasher.update(verifier.as_bytes());

        let hash = hasher.finalize();

        let challenge = base64url_encode(&hash);

        assert_eq!(challenge, "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");

    }

}

