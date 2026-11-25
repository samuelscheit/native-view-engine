//! Constant-time Bearer token verification and OS CSPRNG token generation (zero FNV-1a64).

use subtle::ConstantTimeEq;

/// Verifies `provided` against `expected` in constant time. Rejects empty tokens.
#[inline]
pub fn verify_bearer_token(expected: &str, provided: &str) -> bool {
    let a = expected.as_bytes();
    let b = provided.as_bytes();
    if a.is_empty() || a.len() != b.len() {
        return false;
    }
    bool::from(a.ct_eq(b))
}

/// Generates a 128-bit cryptographically random Bearer token (`nve_<32 lowercase hex chars>`).
pub fn generate_bearer_token() -> String {
    let mut entropy = [0u8; 16];
    getrandom::fill(&mut entropy).expect("OS CSPRNG unavailable for Bearer token generation");
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(4 + 32);
    out.push_str("nve_");
    for byte in entropy {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0F) as usize] as char);
    }
    out
}
