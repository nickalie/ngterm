//! The Android app's self-update check of a GitHub release: [`release_checksum`] verifies the release's `SHA256SUMS`
//! against its `SHA256SUMS.minisig` with the key desktop builds trust (`ngterm-update`'s `trust/minisign.pub`), and
//! hands back the checksum of the one asset the app downloads. It makes the same checks `ngterm-update` makes before
//! a desktop update, so a release a desktop build refuses is refused here too.

use minisign_verify::{PublicKey, Signature};

/// The key a release's checksum manifest must be signed with; empty means unsigned releases are taken as they are.
const TRUSTED_KEY: &str = include_str!("../../ngterm-update/trust/minisign.pub");

/// Why a release's checksums can't be trusted.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum ReleaseError {
    /// The signature is missing, broken, by another key, or names another release.
    #[error("the release's signature doesn't verify")]
    Signature,
    /// The manifest lists the asset not exactly once, or with a malformed checksum.
    #[error("the release lists no single valid checksum for {asset}")]
    Checksum { asset: String },
}

/// The lowercase hex SHA-256 that release `version`'s `SHA256SUMS` (`manifest`) lists for `asset`, once `signature`
/// (`SHA256SUMS.minisig`) verifies the manifest with the trusted key and its trusted comment names this release
/// (`ngterm v<version> SHA256SUMS`), so a genuine signature can't be replayed onto another release.
///
/// # Errors
///
/// [`ReleaseError::Signature`] when the signature doesn't hold, [`ReleaseError::Checksum`] when the asset has no
/// single well-formed checksum.
#[uniffi::export]
#[expect(
    clippy::needless_pass_by_value,
    reason = "UniFFI passes the foreign side's arguments owned"
)]
pub fn release_checksum(
    manifest: Vec<u8>,
    signature: Vec<u8>,
    version: String,
    asset: String,
) -> Result<String, ReleaseError> {
    let key = trusted_key(TRUSTED_KEY)?;
    checksum(key.as_ref(), &manifest, &signature, &version, &asset)
}

fn trusted_key(text: &str) -> Result<Option<PublicKey>, ReleaseError> {
    if text.trim().is_empty() {
        return Ok(None);
    }
    PublicKey::decode(text.trim_start())
        .map(Some)
        .map_err(|_| ReleaseError::Signature)
}

fn checksum(
    key: Option<&PublicKey>,
    manifest: &[u8],
    signature: &[u8],
    version: &str,
    asset: &str,
) -> Result<String, ReleaseError> {
    if let Some(key) = key {
        let signature = std::str::from_utf8(signature).map_err(|_| ReleaseError::Signature)?;
        let signature = Signature::decode(signature).map_err(|_| ReleaseError::Signature)?;
        key.verify(manifest, &signature, false)
            .map_err(|_| ReleaseError::Signature)?;
        if signature.trusted_comment() != format!("ngterm v{version} SHA256SUMS") {
            return Err(ReleaseError::Signature);
        }
    }
    let missing = || ReleaseError::Checksum {
        asset: asset.to_owned(),
    };
    let manifest = std::str::from_utf8(manifest).map_err(|_| missing())?;
    let hashes: Vec<_> = manifest
        .lines()
        .filter_map(|line| {
            let (hash, file) = line.split_once("  ").or_else(|| line.split_once(" *"))?;
            (file == asset).then_some(hash)
        })
        .collect();
    match hashes.as_slice() {
        [hash] if hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()) => Ok(hash.to_ascii_lowercase()),
        _ => Err(missing()),
    }
}

#[cfg(test)]
mod tests {
    //! Over `ngterm-update`'s minisign fixtures (two test-only keys, a manifest and its signatures; that crate's tests
    //! say how they were made), so both updaters are held to the same signatures.

    use super::*;

    const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../ngterm-update/src/tests/minisign/");
    const ASSET: &str = "ngterm-0.2.0-linux-x86_64.tar.gz";
    const HASH: &str = "2b0e6d1b5d1b3c4e0f9a8d7c6b5a49382716051f2e3d4c5b6a79881726354453";

    fn fixture(name: &str) -> Vec<u8> {
        std::fs::read(format!("{FIXTURES}{name}")).unwrap()
    }

    fn key(name: &str) -> Option<PublicKey> {
        trusted_key(&String::from_utf8(fixture(name)).unwrap()).unwrap()
    }

    #[test]
    fn a_signed_manifest_gives_the_assets_checksum() {
        let sums = fixture("SHA256SUMS");
        let signature = fixture("SHA256SUMS.minisig");
        assert_eq!(
            checksum(key("test.pub").as_ref(), &sums, &signature, "0.2.0", ASSET).unwrap(),
            HASH
        );
    }

    #[test]
    fn a_signature_by_another_key_tampered_or_for_another_release_is_refused() {
        let sums = fixture("SHA256SUMS");
        let signature = fixture("SHA256SUMS.minisig");
        let test = key("test.pub");
        let refused = |key: Option<&PublicKey>, manifest: &[u8], signature: &[u8], version: &str| {
            assert_eq!(
                checksum(key, manifest, signature, version, ASSET),
                Err(ReleaseError::Signature)
            );
        };
        refused(key("other.pub").as_ref(), &sums, &signature, "0.2.0");
        refused(test.as_ref(), &sums, &fixture("wrong-key.minisig"), "0.2.0");
        let mut tampered = sums.clone();
        tampered[0] ^= 1;
        refused(test.as_ref(), &tampered, &signature, "0.2.0");
        refused(test.as_ref(), &sums, &fixture("other-release.minisig"), "0.2.0");
        refused(test.as_ref(), &sums, &signature, "0.3.0");
        refused(test.as_ref(), &sums, b"", "0.2.0");
        refused(test.as_ref(), &sums, b"not a signature", "0.2.0");
        refused(test.as_ref(), &sums, &[0xff, 0xfe], "0.2.0");
    }

    #[test]
    fn an_asset_needs_exactly_one_well_formed_checksum() {
        let missing = |manifest: &str| {
            assert_eq!(
                checksum(None, manifest.as_bytes(), b"", "0.2.0", "a.apk"),
                Err(ReleaseError::Checksum { asset: "a.apk".into() })
            );
        };
        let hash = "AB".repeat(32);
        assert_eq!(
            checksum(None, format!("{hash} *a.apk\n").as_bytes(), b"", "0.2.0", "a.apk").unwrap(),
            "ab".repeat(32)
        );
        missing("");
        missing(&format!("{hash}  b.apk\n"));
        missing(&format!("{hash}  a.apk\n{hash}  a.apk\n"));
        missing("abc  a.apk\n");
        missing(&format!("{}  a.apk\n", "g".repeat(64)));
        assert!(checksum(None, &[0xff], b"", "0.2.0", "a.apk").is_err());
    }

    #[test]
    fn the_trusted_key_is_the_one_desktop_builds_trust() {
        assert!(trusted_key(TRUSTED_KEY).unwrap().is_some());
        assert!(trusted_key(" \n").unwrap().is_none());
        assert!(matches!(
            trusted_key("untrusted comment: x\nnot base64\n"),
            Err(ReleaseError::Signature)
        ));
        // the shipped key refuses a manifest signed by a test key
        assert_eq!(
            release_checksum(
                fixture("SHA256SUMS"),
                fixture("SHA256SUMS.minisig"),
                "0.2.0".into(),
                ASSET.into()
            ),
            Err(ReleaseError::Signature)
        );
    }
}
