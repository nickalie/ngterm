//! The app's ssh keys: [`generate`] makes an ed25519 key, [`import`] reads one the user brings (OpenSSH, PKCS#1 or
//! PKCS#8, with its passphrase), and [`load`] turns the bytes a [`crate::KeyVault`] hands over back into a key for a
//! connection.
//!
//! The private bytes the app stores are always an unencrypted OpenSSH key: the Android Keystore wraps them, so an
//! imported key loses its passphrase here, once. Every private byte this module reads or writes on its own side is
//! wiped once used ([`zeroize::Zeroizing`]); the bytes returned in a [`KeyPair`] are the app's to wrap and wipe.

use ::russh::keys::ssh_key::LineEnding;
use ::russh::keys::{Algorithm, HashAlg, PrivateKey, decode_secret_key};

/// A private key as the app stores it, with what it shows of it.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct KeyPair {
    /// The unencrypted OpenSSH private key, for the app to wrap with the Keystore and hand back through
    /// [`crate::KeyVault::load`].
    pub private_key: Vec<u8>,
    /// The public key as an `authorized_keys` line (`ssh-ed25519 AAAA… comment`).
    pub authorized_key: String,
    /// The public key's OpenSSH `SHA256:` fingerprint.
    pub fingerprint: String,
}

/// Why a key could not be made or read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum KeyError {
    /// The key is encrypted and no passphrase was given.
    #[error("the key is protected by a passphrase")]
    PassphraseRequired,
    /// The passphrase does not open the key.
    #[error("the passphrase is wrong")]
    WrongPassphrase,
    /// The bytes are not a private key this app can use.
    #[error("{reason}")]
    Invalid { reason: String },
}

/// A new ed25519 key whose public half carries `comment`.
///
/// # Errors
///
/// [`KeyError::Invalid`] if the key can't be generated or encoded (the system's random source failed).
pub fn generate(comment: &str) -> Result<KeyPair, KeyError> {
    let mut key = PrivateKey::random(&mut ::russh::keys::key::safe_rng(), Algorithm::Ed25519).map_err(invalid)?;
    key.set_comment(comment);
    pair(&key)
}

/// The key in `bytes` (OpenSSH, PKCS#1 or PKCS#8 PEM), opened with `passphrase` when it is encrypted, re-encoded as
/// the unencrypted OpenSSH key the app stores.
///
/// # Errors
///
/// [`KeyError::PassphraseRequired`] for an encrypted key with no passphrase, [`KeyError::WrongPassphrase`] when the
/// passphrase doesn't open it, [`KeyError::Invalid`] for anything that isn't a usable private key.
pub fn import(bytes: &[u8], passphrase: Option<&str>) -> Result<KeyPair, KeyError> {
    let text = std::str::from_utf8(bytes).map_err(|_| KeyError::Invalid {
        reason: "the key is not text".to_owned(),
    })?;
    let passphrase = passphrase.filter(|passphrase| !passphrase.is_empty());
    let key = decode_secret_key(text, passphrase).map_err(|error| match error {
        ::russh::keys::Error::KeyIsEncrypted => KeyError::PassphraseRequired,
        _ if is_encrypted(text) => {
            if passphrase.is_some() {
                KeyError::WrongPassphrase
            } else {
                KeyError::PassphraseRequired
            }
        }
        error => invalid(error),
    })?;
    if key.is_encrypted() {
        return Err(KeyError::PassphraseRequired);
    }
    pair(&key)
}

/// The key a [`crate::KeyVault`] handed over, as [`generate`] or [`import`] produced it. The caller wipes `bytes`
/// (holds them in [`zeroize::Zeroizing`]); the key wipes itself when dropped.
///
/// # Errors
///
/// [`KeyError`] as [`import`] gives it with no passphrase.
pub fn load(bytes: &[u8]) -> Result<PrivateKey, KeyError> {
    let text = std::str::from_utf8(bytes).map_err(|_| KeyError::Invalid {
        reason: "the key is not text".to_owned(),
    })?;
    let key = decode_secret_key(text, None).map_err(|error| match error {
        ::russh::keys::Error::KeyIsEncrypted => KeyError::PassphraseRequired,
        error => invalid(error),
    })?;
    if key.is_encrypted() {
        return Err(KeyError::PassphraseRequired);
    }
    Ok(key)
}

/// Whether the PEM text holds an encrypted key: an OpenSSH key with a cipher, or an encrypted PKCS#8 or PKCS#5 one.
fn is_encrypted(text: &str) -> bool {
    PrivateKey::from_openssh(text).is_ok_and(|key| key.is_encrypted())
        || text.contains("-----BEGIN ENCRYPTED PRIVATE KEY-----")
        || text.contains("Proc-Type: 4,ENCRYPTED")
}

fn pair(key: &PrivateKey) -> Result<KeyPair, KeyError> {
    let private = key.to_openssh(LineEnding::LF).map_err(invalid)?;
    let authorized_key = key.public_key().to_openssh().map_err(invalid)?;
    Ok(KeyPair {
        private_key: private.as_bytes().to_vec(),
        authorized_key,
        fingerprint: key.fingerprint(HashAlg::Sha256).to_string(),
    })
}

fn invalid(error: impl std::fmt::Display) -> KeyError {
    KeyError::Invalid {
        reason: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use ::russh::keys::PublicKey;

    use super::*;

    fn encrypted(key: &PrivateKey, passphrase: &str) -> Vec<u8> {
        let mut rng = ::russh::keys::key::safe_rng();
        let locked = key.encrypt(&mut rng, passphrase).unwrap();
        locked.to_openssh(LineEnding::LF).unwrap().as_bytes().to_vec()
    }

    #[test]
    fn a_generated_key_is_ed25519_and_its_parts_agree() {
        let pair = generate("phone").unwrap();

        let key = load(&pair.private_key).unwrap();
        assert_eq!(key.algorithm(), Algorithm::Ed25519);
        assert!(pair.authorized_key.starts_with("ssh-ed25519 "));
        assert!(pair.authorized_key.ends_with(" phone"));
        let public = PublicKey::from_openssh(&pair.authorized_key).unwrap();
        assert_eq!(public.key_data(), key.public_key().key_data());
        assert_eq!(pair.fingerprint, key.fingerprint(HashAlg::Sha256).to_string());
        assert!(pair.fingerprint.starts_with("SHA256:"));
    }

    #[test]
    fn two_generated_keys_differ() {
        assert_ne!(generate("a").unwrap().private_key, generate("a").unwrap().private_key);
    }

    #[test]
    fn an_unencrypted_key_imports_as_it_is() {
        let pair = generate("laptop").unwrap();

        let imported = import(&pair.private_key, None).unwrap();

        assert_eq!(imported, pair);
    }

    #[test]
    fn an_encrypted_key_imports_with_its_passphrase_and_is_stored_unencrypted() {
        let pair = generate("laptop").unwrap();
        let key = load(&pair.private_key).unwrap();

        let imported = import(&encrypted(&key, "secret"), Some("secret")).unwrap();

        assert_eq!(imported.authorized_key, pair.authorized_key);
        assert!(!load(&imported.private_key).unwrap().is_encrypted());
    }

    #[test]
    fn an_encrypted_key_needs_its_passphrase() {
        let key = load(&generate("x").unwrap().private_key).unwrap();
        let locked = encrypted(&key, "secret");

        assert_eq!(import(&locked, None), Err(KeyError::PassphraseRequired));
        assert_eq!(import(&locked, Some("")), Err(KeyError::PassphraseRequired));
        assert_eq!(import(&locked, Some("wrong")), Err(KeyError::WrongPassphrase));
        assert_eq!(load(&locked).err(), Some(KeyError::PassphraseRequired));
    }

    #[test]
    fn a_pkcs8_key_imports() {
        let pkcs8 = "-----BEGIN PRIVATE KEY-----\n\
                     MC4CAQAwBQYDK2VwBCIEINTuctv5E1hK1bbY8fdp+K06/nwoy/HU++CXqI9EdVhC\n\
                     -----END PRIVATE KEY-----\n";

        let imported = import(pkcs8.as_bytes(), None).unwrap();

        assert!(imported.authorized_key.starts_with("ssh-ed25519 "));
    }

    #[test]
    fn what_is_not_a_key_is_invalid() {
        assert!(matches!(import(b"hello", None), Err(KeyError::Invalid { .. })));
        assert!(matches!(import(&[0xff, 0xfe], None), Err(KeyError::Invalid { .. })));
        assert!(matches!(load(b"hello"), Err(KeyError::Invalid { .. })));
    }
}
