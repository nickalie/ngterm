//! One named asset of a stated release, for a caller that installs it elsewhere (`ngtermctl server install`, which sends
//! this build's Linux release archive to a host): the checksum the release was published with, read from its
//! `SHA256SUMS` once the build's trusted key verified it, as an update verifies it ([`Source::published_checksum`]),
//! and a bounded download into a private file ([`Source::fetch_asset`]).

use std::fmt::Write as _;
use std::fs::{File, OpenOptions};
use std::io::{self, Read as _};
use std::path::Path;

use semver::Version;
use sha2::{Digest as _, Sha256};

use crate::download::{self, METADATA_LIMIT, SIGNATURE, SIGNATURE_LIMIT, bounded_read};
use crate::{Error, Format, Source, Trust, error};

/// The Linux release archive of `version` for `architecture` (`x86_64` or `aarch64`).
///
/// # Errors
/// Returns an error for an architecture no Linux archive is built for.
pub fn linux_archive(version: &Version, architecture: &str) -> Result<String, Error> {
    Format::LinuxArchive.asset(version, architecture)
}

/// The lowercase SHA-256 of `path`'s contents.
///
/// # Errors
/// Returns the file's read error.
pub fn file_sha256(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hash.update(&buffer[..read]);
    }
    Ok(hash.finalize().iter().fold(String::new(), |mut hex, byte| {
        let _ = write!(hex, "{byte:02x}");
        hex
    }))
}

/// Creates `path` readable and writable by its owner alone; it must not exist yet.
///
/// # Errors
/// Returns the file system's error.
pub fn create_private(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(path)
}

impl Source {
    fn release_url(&self, version: &Version, name: &str) -> String {
        format!("{}v{version}/{name}", self.downloads)
    }

    /// The checksum release `version` was published with for `asset`: its `SHA256SUMS`, verified against
    /// `SHA256SUMS.minisig` when the build trusts a signing key, must name the asset exactly once.
    ///
    /// # Errors
    /// Returns transport, size, signature or checksum errors.
    pub fn published_checksum(&self, version: &Version, asset: &str, trust: &Trust) -> Result<String, Error> {
        let fetch = |name: &str, limit: u64| -> Result<Vec<u8>, Error> {
            let mut response = self.agent.get(&self.release_url(version, name)).call()?;
            bounded_read(response.body_mut().as_reader(), limit)
        };
        let manifest = fetch("SHA256SUMS", METADATA_LIMIT)?;
        let signature = match trust.minisign {
            Some(_) => Some(fetch(SIGNATURE, SIGNATURE_LIMIT)?),
            None => None,
        };
        let manifest = download::verified_manifest(trust, version, manifest, signature.as_deref())?;
        download::checksum(&manifest, asset)
    }

    /// The download URL of `asset` in release `version`.
    #[must_use]
    pub fn asset_url(&self, version: &Version, asset: &str) -> String {
        self.release_url(version, asset)
    }

    /// Downloads `asset` of release `version` into `target`, created private ([`create_private`]), refusing more than
    /// `limit` bytes. The caller checks the file against [`Source::published_checksum`].
    ///
    /// # Errors
    /// Returns transport, file system or size errors; a partial file stays for the caller to remove.
    pub fn fetch_asset(&self, version: &Version, asset: &str, target: &Path, limit: u64) -> Result<(), Error> {
        let mut response = self.agent.get(&self.release_url(version, asset)).call()?;
        let mut file = create_private(target)?;
        let copied = io::copy(&mut response.body_mut().as_reader().take(limit + 1), &mut file)?;
        if copied > limit {
            return Err(error(format!("larger than {limit} bytes")));
        }
        file.sync_all()?;
        Ok(())
    }
}

#[cfg(test)]
#[path = "release_asset_tests.rs"]
mod tests;
