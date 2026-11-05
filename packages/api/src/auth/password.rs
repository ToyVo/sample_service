//! Password hashing with argon2id (the crate's OWASP-recommended defaults),
//! stored as PHC strings. Hashing is CPU-bound, so callers run it on
//! `spawn_blocking`.

use std::sync::OnceLock;

use argon2::Argon2;
use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};

/// Hash a password into an argon2id PHC string.
///
/// # Errors
///
/// Returns the hasher's error message (only on internal failure).
pub fn hash_password(password: &str) -> Result<String, String> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|err| err.to_string())
}

/// Check a password against a stored PHC string. Malformed hashes never
/// verify.
#[must_use]
pub fn verify_password(password: &str, phc: &str) -> bool {
    PasswordHash::new(phc).is_ok_and(|parsed| {
        Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok()
    })
}

/// Burn the same argon2 work as a real verification so sign-in attempts for
/// unknown usernames take as long as wrong passwords (no timing oracle).
pub fn verify_dummy_password(password: &str) {
    static DUMMY: OnceLock<String> = OnceLock::new();
    let phc = DUMMY.get_or_init(|| hash_password("dummy-password").unwrap_or_default());
    let _ = verify_password(password, phc);
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn passwords_verify_only_against_their_own_hash() {
        let phc = hash_password("correct horse").expect("hashes");
        assert!(phc.starts_with("$argon2id$"));
        assert!(verify_password("correct horse", &phc));
        assert!(!verify_password("wrong horse", &phc));
        assert!(!verify_password("correct horse", "not a phc string"));
        // Salts differ, so equal passwords never share a hash.
        assert_ne!(phc, hash_password("correct horse").expect("hashes"));
    }
}
