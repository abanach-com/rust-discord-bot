use sha2::{Digest, Sha256};
use std::sync::RwLock;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

/// Pure-Rust RFC 2104 HMAC-SHA256 implementation
pub fn compute_hmac_sha256(key: &[u8], data: &[u8]) -> [u8; 32] {
    let mut k = [0u8; 64];
    if key.len() > 64 {
        let hash = Sha256::digest(key);
        k[..32].copy_from_slice(&hash);
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let mut ipad = [0x36u8; 64];
    let mut opad = [0x5cu8; 64];
    for i in 0..64 {
        ipad[i] ^= k[i];
        opad[i] ^= k[i];
    }
    let mut inner_hasher = Sha256::new();
    inner_hasher.update(ipad);
    inner_hasher.update(data);
    let inner_hash = inner_hasher.finalize();

    let mut outer_hasher = Sha256::new();
    outer_hasher.update(opad);
    outer_hasher.update(inner_hash);
    outer_hasher.finalize().into()
}

pub fn verify_hmac_sha256(secret: &str, body: &[u8], expected_hex_or_prefixed: &str) -> bool {
    let expected_hex = if let Some(stripped) = expected_hex_or_prefixed.strip_prefix("sha256=") {
        stripped
    } else {
        expected_hex_or_prefixed
    };

    let computed = compute_hmac_sha256(secret.as_bytes(), body);
    let computed_hex = hex::encode(computed);

    // Constant-time comparison
    if computed_hex.len() != expected_hex.len() {
        return false;
    }

    let mut diff = 0u8;
    for (a, b) in computed_hex.bytes().zip(expected_hex.bytes()) {
        diff |= a ^ b;
    }
    diff == 0
}

pub fn verify_secret_token(expected_token: &str, provided_token: &str) -> bool {
    if expected_token.len() != provided_token.len() {
        return false;
    }
    let mut diff = 0u8;
    for (a, b) in expected_token.bytes().zip(provided_token.bytes()) {
        diff |= a ^ b;
    }
    diff == 0
}

// ==========================================
// Circuit Breaker for Delivery Broker
// ==========================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CircuitState {
    Closed,
    Open,
    HalfOpen,
}

pub struct CircuitBreaker {
    failures: AtomicU32,
    failure_threshold: u32,
    reset_timeout: Duration,
    last_failure: RwLock<Option<Instant>>,
}

impl CircuitBreaker {
    pub fn new(failure_threshold: u32, reset_timeout: Duration) -> Self {
        Self {
            failures: AtomicU32::new(0),
            failure_threshold,
            reset_timeout,
            last_failure: RwLock::new(None),
        }
    }

    pub fn state(&self) -> CircuitState {
        let fails = self.failures.load(Ordering::Relaxed);
        if fails < self.failure_threshold {
            return CircuitState::Closed;
        }

        let last = *self.last_failure.read().unwrap();
        if let Some(t) = last
            && t.elapsed() >= self.reset_timeout
        {
            return CircuitState::HalfOpen;
        }
        CircuitState::Open
    }

    pub fn record_success(&self) {
        self.failures.store(0, Ordering::Relaxed);
        *self.last_failure.write().unwrap() = None;
    }

    pub fn record_failure(&self) {
        self.failures.fetch_add(1, Ordering::Relaxed);
        *self.last_failure.write().unwrap() = Some(Instant::now());
    }

    pub fn allow_request(&self) -> bool {
        self.state() != CircuitState::Open
    }
}
