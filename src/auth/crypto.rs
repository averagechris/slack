//! Cryptographic operations for export/import functionality
//!
//! Uses Argon2id for key derivation and AES-256-GCM for encryption

use aes_gcm::{
    aead::{Aead, KeyInit, OsRng},
    Aes256Gcm, Nonce,
};
use argon2::{Algorithm, Argon2, Params, Version};
use rand::RngCore;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CryptoError {
    #[error("Encryption failed: {0}")]
    EncryptionFailed(String),
    #[error("Decryption failed: {0}")]
    DecryptionFailed(String),
    #[error("Key derivation failed: {0}")]
    KeyDerivationFailed(String),
    #[error("Invalid passphrase")]
    InvalidPassphrase,
}

pub type Result<T> = std::result::Result<T, CryptoError>;

/// KDF parameters for Argon2id
#[derive(Debug, Clone)]
pub struct KdfParams {
    pub salt: Vec<u8>,
    pub memory_cost: u32,
    pub time_cost: u32,
    pub parallelism: u32,
}

impl Default for KdfParams {
    fn default() -> Self {
        Self {
            salt: Vec::new(),
            memory_cost: 19456, // 19 MiB
            time_cost: 2,
            parallelism: 1,
        }
    }
}

/// Encrypted data with nonce
#[derive(Debug, Clone)]
pub struct EncryptedData {
    pub nonce: Vec<u8>,
    pub ciphertext: Vec<u8>,
}

/// Derive encryption key from passphrase using Argon2id
///
/// Uses the KDF parameters provided by the caller (e.g. read from an export
/// file header) rather than crate defaults, so old exports remain decryptable
/// even if the `argon2` crate defaults change.
pub fn derive_key(passphrase: &str, params: &KdfParams) -> Result<[u8; 32]> {
    if passphrase.is_empty() {
        return Err(CryptoError::InvalidPassphrase);
    }

    let argon2_params = Params::new(
        params.memory_cost,
        params.time_cost,
        params.parallelism,
        Some(32),
    )
    .map_err(|e| CryptoError::KeyDerivationFailed(format!("Invalid KDF parameters: {}", e)))?;

    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, argon2_params);

    let mut key = [0u8; 32];
    argon2
        .hash_password_into(passphrase.as_bytes(), &params.salt, &mut key)
        .map_err(|e| CryptoError::KeyDerivationFailed(e.to_string()))?;

    Ok(key)
}

/// Generate random salt for KDF
pub fn generate_salt() -> Vec<u8> {
    let mut salt = vec![0u8; 16];
    OsRng.fill_bytes(&mut salt);
    salt
}

/// Encrypt data with AES-256-GCM
pub fn encrypt(plaintext: &[u8], key: &[u8; 32]) -> Result<EncryptedData> {
    let cipher = Aes256Gcm::new(key.into());

    // Generate random nonce
    let mut nonce_bytes = [0u8; 12];
    OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    // Encrypt
    let ciphertext = cipher
        .encrypt(nonce, plaintext)
        .map_err(|e| CryptoError::EncryptionFailed(e.to_string()))?;

    Ok(EncryptedData {
        nonce: nonce_bytes.to_vec(),
        ciphertext,
    })
}

/// Decrypt data with AES-256-GCM
pub fn decrypt(encrypted: &EncryptedData, key: &[u8; 32]) -> Result<Vec<u8>> {
    if encrypted.nonce.len() != 12 {
        return Err(CryptoError::DecryptionFailed(
            "Invalid nonce length".to_string(),
        ));
    }

    let cipher = Aes256Gcm::new(key.into());
    let nonce = Nonce::from_slice(&encrypted.nonce);

    let plaintext = cipher
        .decrypt(nonce, encrypted.ciphertext.as_ref())
        .map_err(|e| CryptoError::DecryptionFailed(e.to_string()))?;

    Ok(plaintext)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_derive_key_deterministic() {
        let passphrase = "test_password";
        let params = KdfParams {
            salt: vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16],
            memory_cost: 19456,
            time_cost: 2,
            parallelism: 1,
        };

        let key1 = derive_key(passphrase, &params).unwrap();
        let key2 = derive_key(passphrase, &params).unwrap();

        assert_eq!(
            key1, key2,
            "Same passphrase and salt should produce same key"
        );
    }

    #[test]
    fn test_derive_key_empty_passphrase() {
        let params = KdfParams::default();
        let result = derive_key("", &params);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            CryptoError::InvalidPassphrase
        ));
    }

    #[test]
    fn test_encrypt_decrypt_round_trip() {
        let passphrase = "test_password";
        let plaintext = b"Hello, World!";

        let params = KdfParams {
            salt: generate_salt(),
            ..Default::default()
        };

        let key = derive_key(passphrase, &params).unwrap();

        let encrypted = encrypt(plaintext, &key).unwrap();
        let decrypted = decrypt(&encrypted, &key).unwrap();

        assert_eq!(plaintext, decrypted.as_slice());
    }

    #[test]
    fn test_decrypt_wrong_key() {
        let plaintext = b"Hello, World!";

        let params1 = KdfParams {
            salt: generate_salt(),
            ..Default::default()
        };
        let key1 = derive_key("password1", &params1).unwrap();

        let params2 = KdfParams {
            salt: generate_salt(),
            ..Default::default()
        };
        let key2 = derive_key("password2", &params2).unwrap();

        let encrypted = encrypt(plaintext, &key1).unwrap();
        let result = decrypt(&encrypted, &key2);

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            CryptoError::DecryptionFailed(_)
        ));
    }

    #[test]
    fn test_generate_salt_unique() {
        let salt1 = generate_salt();
        let salt2 = generate_salt();

        assert_ne!(salt1, salt2, "Generated salts should be unique");
    }

    #[test]
    fn test_nonce_uniqueness() {
        let key = [0u8; 32];
        let plaintext = b"test";

        let encrypted1 = encrypt(plaintext, &key).unwrap();
        let encrypted2 = encrypt(plaintext, &key).unwrap();

        assert_ne!(
            encrypted1.nonce, encrypted2.nonce,
            "Nonces should be unique"
        );
    }

    #[test]
    fn test_round_trip_with_non_default_params() {
        let passphrase = "test_password";
        let plaintext = b"sensitive data";

        let params = KdfParams {
            salt: generate_salt(),
            memory_cost: 8192, // non-default
            time_cost: 3,      // non-default
            parallelism: 2,    // non-default
        };

        let key = derive_key(passphrase, &params).unwrap();
        let encrypted = encrypt(plaintext, &key).unwrap();

        // Re-derive with the same (non-default) params, as import would
        let key2 = derive_key(passphrase, &params).unwrap();
        let decrypted = decrypt(&encrypted, &key2).unwrap();
        assert_eq!(plaintext, decrypted.as_slice());
    }

    #[test]
    fn test_non_default_params_produce_different_key() {
        let passphrase = "test_password";
        let salt = generate_salt();

        let default_params = KdfParams {
            salt: salt.clone(),
            ..Default::default()
        };
        let custom_params = KdfParams {
            salt,
            memory_cost: 8192,
            time_cost: 3,
            parallelism: 2,
        };

        let key_default = derive_key(passphrase, &default_params).unwrap();
        let key_custom = derive_key(passphrase, &custom_params).unwrap();

        assert_ne!(
            key_default, key_custom,
            "Different KDF params must produce different keys (params must not be ignored)"
        );
    }

    #[test]
    fn test_decrypt_fails_gracefully_with_mismatched_params() {
        let passphrase = "test_password";
        let plaintext = b"secret";
        let salt = generate_salt();

        let encrypt_params = KdfParams {
            salt: salt.clone(),
            memory_cost: 8192,
            time_cost: 3,
            parallelism: 2,
        };
        let key = derive_key(passphrase, &encrypt_params).unwrap();
        let encrypted = encrypt(plaintext, &key).unwrap();

        // Derive with mismatched (default) params: must error, not panic
        let wrong_params = KdfParams {
            salt,
            ..Default::default()
        };
        let wrong_key = derive_key(passphrase, &wrong_params).unwrap();
        let result = decrypt(&encrypted, &wrong_key);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            CryptoError::DecryptionFailed(_)
        ));
    }

    #[test]
    fn test_default_params_round_trip_still_works() {
        let passphrase = "test_password";
        let plaintext = b"legacy export";

        let params = KdfParams {
            salt: generate_salt(),
            ..Default::default()
        };

        let key = derive_key(passphrase, &params).unwrap();
        let encrypted = encrypt(plaintext, &key).unwrap();
        let decrypted = decrypt(&encrypted, &key).unwrap();
        assert_eq!(plaintext, decrypted.as_slice());
    }

    #[test]
    fn test_derive_key_invalid_params_error_not_panic() {
        // parallelism of 0 is invalid for Argon2
        let params = KdfParams {
            salt: generate_salt(),
            memory_cost: 8192,
            time_cost: 1,
            parallelism: 0,
        };
        let result = derive_key("password", &params);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            CryptoError::KeyDerivationFailed(_)
        ));
    }
}
