use std::fs::{self, File};
use std::io::{Read as _, Write as _};
use std::path::Path;
use std::time::Duration;

use semver::Version;
use serde::Deserialize;
use sha2::{Digest as _, Sha256};

use crate::{Error, Installation, Prepared, Trust, error};

const API: &str = "https://api.github.com/repos/nickalie/ngterm/releases/latest";
pub(crate) const DOWNLOADS: &str = "https://github.com/nickalie/ngterm/releases/download/";
pub(crate) const METADATA_LIMIT: u64 = 2 * 1024 * 1024;
const ASSET_LIMIT: u64 = 2 * 1024 * 1024 * 1024;
// a minisign signature is four short lines
pub(crate) const SIGNATURE_LIMIT: u64 = 4 * 1024;
/// GitHub's primary limit resets each hour, so a longer wait comes from a broken answer.
const LIMIT_WAIT_MAX: Duration = Duration::from_hours(1);
/// GitHub asks for at least a minute's wait after a secondary limit whose answer gives no wait.
const SECONDARY_WAIT: Duration = Duration::from_secs(60);

#[derive(Deserialize)]
pub(crate) struct Metadata {
    pub tag_name: String,
    pub draft: bool,
    pub prerelease: bool,
    pub assets: Vec<Asset>,
}

#[derive(Deserialize)]
pub(crate) struct Asset {
    pub name: String,
    pub browser_download_url: String,
    pub size: u64,
}

/// A stable release with exactly the asset required by the running installation, its checksum manifest and, when
/// the build trusts a signing key, the manifest's signature.
#[derive(Debug, Clone)]
pub struct Release {
    pub version: Version,
    pub(crate) asset: String,
    pub(crate) url: String,
    pub(crate) checksum_url: String,
    pub(crate) signature_url: Option<String>,
    pub(crate) size: u64,
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .https_only(true)
        .timeout_global(Some(Duration::from_mins(15)))
        .timeout_connect(Some(Duration::from_secs(20)))
        .timeout_per_read(Some(Duration::from_secs(30)))
        .build()
        .into()
}

/// Where releases are looked up and downloaded from. Shipped builds have only [`Source::github`]: GitHub's API and
/// release downloads, over HTTPS alone. Tests read a local release through `fakes`, a feature shipped builds never
/// enable.
#[derive(Clone)]
pub struct Source {
    pub(crate) api: String,
    pub(crate) downloads: String,
    pub(crate) agent: ureq::Agent,
}

impl Source {
    /// GitHub's releases of ngterm.
    #[must_use]
    pub fn github() -> Self {
        #[cfg(coverage)]
        if let Some(source) = crate::stand_in::source() {
            return source;
        }
        Self {
            api: API.into(),
            downloads: DOWNLOADS.into(),
            agent: agent(),
        }
    }

    /// Queries the latest stable release. Never installs prereleases, downgrades or the running version. With a
    /// trusted signing key, a release without `SHA256SUMS.minisig` is refused. After a primary rate limit, the
    /// GitHub source retries once with a local token; only a [`Trigger::Manual`] check asks `gh` for it.
    ///
    /// # Errors
    /// Returns network, metadata or incompatible release errors.
    pub fn check(
        &self,
        installation: &Installation,
        current: &str,
        trust: &Trust,
        trigger: Trigger,
    ) -> Result<Option<Release>, Error> {
        self.check_with(installation, current, trust, || {
            crate::github_auth::local_token(trigger)
        })
    }

    /// [`Source::check`] with the token lookup that it uses after a rate limit.
    pub(crate) fn check_with(
        &self,
        installation: &Installation,
        current: &str,
        trust: &Trust,
        lookup: impl FnOnce() -> Option<String>,
    ) -> Result<Option<Release>, Error> {
        let mut response = metadata_response(&self.agent, &self.api, || github_only(&self.api, lookup))?;
        let bytes = bounded_read(response.body_mut().as_reader(), METADATA_LIMIT)?;
        select(
            &serde_json::from_slice(&bytes)?,
            &self.downloads,
            installation,
            current,
            trust,
        )
    }
}

/// Who starts a release check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    /// A background check: it uses only `GH_TOKEN` or `GITHUB_TOKEN`, never `gh`.
    Automatic,
    /// A check that the user starts: it can also ask `gh` for its token.
    Manual,
}

/// The token lookup runs only for GitHub's own API, so no other source ever gets a local token.
fn github_only(api: &str, lookup: impl FnOnce() -> Option<String>) -> Option<String> {
    #[cfg(coverage)]
    let api = if crate::stand_in::is_api(api) { API } else { api };
    (api == API).then(lookup).flatten()
}

fn metadata_request(
    agent: &ureq::Agent,
    url: &str,
    authorization: Option<ureq::http::HeaderValue>,
) -> Result<ureq::http::Response<ureq::Body>, Error> {
    let mut request = agent
        .get(url)
        .config()
        .http_status_as_error(false)
        .max_redirects(0)
        .build()
        .header("User-Agent", concat!("ngterm/", env!("CARGO_PKG_VERSION")))
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28");
    if let Some(value) = authorization {
        request = request.header("Authorization", value);
    }
    Ok(request.call()?)
}

fn metadata_response(
    agent: &ureq::Agent,
    url: &str,
    credentials: impl FnOnce() -> Option<String>,
) -> Result<ureq::http::Response<ureq::Body>, Error> {
    let mut response = metadata_request(agent, url, None)?;
    if primary_limit(&response)
        && !response.headers().contains_key("retry-after")
        && let Some(token) = credentials()
        && let Ok(mut authorization) = ureq::http::HeaderValue::try_from(format!("Bearer {token}"))
    {
        authorization.set_sensitive(true);
        let wait = limit_wait(&response);
        response = metadata_request(agent, url, Some(authorization))?;
        if matches!(response.status().as_u16(), 401 | 403) && !primary_limit(&response) {
            return Err(limit_error(
                wait,
                " GitHub refused the local token (GH_TOKEN, GITHUB_TOKEN or gh auth token).",
            ));
        }
    }
    if primary_limit(&response) || secondary_limit(&response) {
        return Err(limit_error(limit_wait(&response), ""));
    }
    if unmarked_secondary_limit(&mut response) {
        return Err(limit_error(Some(SECONDARY_WAIT), ""));
    }
    if !response.status().is_success() {
        return Err(ureq::Error::StatusCode(response.status().as_u16()).into());
    }
    Ok(response)
}

fn header_seconds(response: &ureq::http::Response<ureq::Body>, name: &str) -> Option<u64> {
    response
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<u64>().ok())
}

/// The wait that GitHub asks for: `Retry-After`, else the time until `X-RateLimit-Reset`.
fn limit_wait(response: &ureq::http::Response<ureq::Body>) -> Option<Duration> {
    let seconds = match header_seconds(response, "retry-after") {
        Some(seconds) => seconds,
        None => header_seconds(response, "x-ratelimit-reset")?.saturating_sub(server_now(response)),
    };
    Some(Duration::from_secs(seconds).min(LIMIT_WAIT_MAX))
}

/// GitHub's clock from the answer's `Date`, so a wrong local clock does not move the reset.
fn server_now(response: &ureq::http::Response<ureq::Body>) -> u64 {
    response
        .headers()
        .get("date")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| httpdate::parse_http_date(value).ok())
        .unwrap_or_else(std::time::SystemTime::now)
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn limit_error(wait: Option<Duration>, note: &str) -> Error {
    let retry = wait.map_or_else(String::new, |wait| format!(" Retry in {} seconds.", wait.as_secs()));
    Error::RateLimited {
        message: format!(
            "GitHub's API rate limit was reached.{retry}{note} Check local GitHub authentication or use Release Downloads."
        ),
        retry_after: wait,
    }
}

/// GitHub's secondary limit: a refusal with `Retry-After`.
fn secondary_limit(response: &ureq::http::Response<ureq::Body>) -> bool {
    matches!(response.status().as_u16(), 403 | 429) && response.headers().contains_key("retry-after")
}

/// GitHub's secondary limit with no wait in its headers: only the answer's message names it.
fn unmarked_secondary_limit(response: &mut ureq::http::Response<ureq::Body>) -> bool {
    matches!(response.status().as_u16(), 403 | 429)
        && bounded_read(response.body_mut().as_reader(), METADATA_LIMIT).is_ok_and(|body| {
            String::from_utf8_lossy(&body)
                .to_ascii_lowercase()
                .contains("secondary rate limit")
        })
}

fn primary_limit(response: &ureq::http::Response<ureq::Body>) -> bool {
    matches!(response.status().as_u16(), 403 | 429)
        && response
            .headers()
            .get("x-ratelimit-remaining")
            .is_some_and(|value| value == "0")
}

pub(crate) fn select(
    metadata: &Metadata,
    downloads: &str,
    installation: &Installation,
    current: &str,
    trust: &Trust,
) -> Result<Option<Release>, Error> {
    let current = Version::parse(current).map_err(|e| error(e.to_string()))?;
    let version = metadata
        .tag_name
        .strip_prefix('v')
        .and_then(|tag| Version::parse(tag).ok())
        .ok_or_else(|| error("The release has an invalid version tag."))?;
    if metadata.draft
        || metadata.prerelease
        || !version.pre.is_empty()
        || version <= current
        || installation
            .installed_version()
            .is_some_and(|installed| version <= installed)
    {
        return Ok(None);
    }
    let name = installation.format.asset(&version, std::env::consts::ARCH)?;
    let find = |name: &str| -> Result<&Asset, Error> {
        let matches: Vec<_> = metadata.assets.iter().filter(|asset| asset.name == name).collect();
        let [asset] = matches.as_slice() else {
            return Err(error(format!("Release is missing a unique {name}.")));
        };
        if asset.browser_download_url != format!("{downloads}v{version}/{name}") {
            return Err(error("Release asset has an unexpected download URL."));
        }
        Ok(*asset)
    };
    let asset = find(&name)?;
    let checksums = find("SHA256SUMS")?;
    let signature = trust.minisign.is_some().then(|| find(SIGNATURE)).transpose()?;
    if asset.size == 0
        || asset.size > ASSET_LIMIT
        || checksums.size > METADATA_LIMIT
        || signature.is_some_and(|signature| signature.size > SIGNATURE_LIMIT)
    {
        return Err(error("Release asset size is outside the supported range."));
    }
    Ok(Some(Release {
        version,
        asset: name,
        url: asset.browser_download_url.clone(),
        checksum_url: checksums.browser_download_url.clone(),
        signature_url: signature.map(|signature| signature.browser_download_url.clone()),
        size: asset.size,
    }))
}

pub(crate) const SIGNATURE: &str = "SHA256SUMS.minisig";

/// The checksum manifest, once its signature checks out: with a trusted key, `signature` must verify the exact
/// manifest bytes, and its trusted comment must name this release (`ngterm v<version> SHA256SUMS`), so a signature
/// can't be replayed onto another release's manifest. With no key the manifest is taken as it is.
pub(crate) fn verified_manifest(
    trust: &Trust,
    version: &Version,
    manifest: Vec<u8>,
    signature: Option<&[u8]>,
) -> Result<String, Error> {
    if let Some(key) = &trust.minisign {
        let refused = || error("Release signature verification failed. The installation was not changed.");
        let signature = signature.ok_or_else(refused)?;
        let signature = std::str::from_utf8(signature).map_err(|_| refused())?;
        let signature = minisign_verify::Signature::decode(signature).map_err(|_| refused())?;
        key.verify(&manifest, &signature, false).map_err(|_| refused())?;
        if signature.trusted_comment() != format!("ngterm v{version} SHA256SUMS") {
            return Err(refused());
        }
    }
    String::from_utf8(manifest).map_err(|e| error(e.to_string()))
}

pub(crate) fn bounded_read(mut reader: impl std::io::Read, limit: u64) -> Result<Vec<u8>, Error> {
    let mut bytes = Vec::new();
    reader.by_ref().take(limit + 1).read_to_end(&mut bytes)?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > limit {
        return Err(error("Release metadata is too large."));
    }
    Ok(bytes)
}

pub(crate) fn checksum(manifest: &str, name: &str) -> Result<String, Error> {
    let hashes: Vec<_> = manifest
        .lines()
        .filter_map(|line| {
            let (hash, file) = line.split_once("  ").or_else(|| line.split_once(" *"))?;
            (file == name).then_some(hash)
        })
        .collect();
    let [hash] = hashes.as_slice() else {
        return Err(error("Release checksum is missing or ambiguous."));
    };
    if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(error("Release checksum is invalid."));
    }
    Ok(hash.to_ascii_lowercase())
}

impl Source {
    /// Streams and hashes a download, then prepares the complete replacement beside the current installation so the
    /// eventual rename stays on one filesystem. With a trusted signing key, the checksum manifest is verified against
    /// `SHA256SUMS.minisig` before any checksum is read. Returning false from `progress` cancels the download and
    /// removes its stage; dropping a prepared update also removes the staged data.
    ///
    /// # Errors
    /// Returns download, signature, checksum, extraction or filesystem errors; the installation is left untouched.
    pub fn prepare(
        &self,
        installation: &Installation,
        release: &Release,
        trust: &Trust,
        mut progress: impl FnMut(u64, u64) -> bool,
    ) -> Result<Prepared, Error> {
        if !progress(0, release.size) {
            return Err(error("Update download was cancelled."));
        }
        installation.validate_access()?;
        let work = crate::staging::create(installation, &installation.stage_parent()?)?;
        crate::install::record_installation(installation, work.path())?;
        if installation.format != crate::Format::AppImage {
            let recorded = fs::read_to_string(work.path().join("expected-identity"))?;
            let installed = recorded
                .lines()
                .next()
                .and_then(|text| Version::parse(text).ok())
                .ok_or_else(|| error("The installed version identity is invalid."))?;
            if installed >= release.version {
                return Err(error(
                    "The installation was updated while checking for releases. Check for updates again.",
                ));
            }
        }
        let agent = &self.agent;
        let mut manifest = agent.get(&release.checksum_url).call()?;
        let bytes = bounded_read(manifest.body_mut().as_reader(), METADATA_LIMIT)?;
        let signature = match (&trust.minisign, &release.signature_url) {
            (None, _) => None,
            (Some(_), None) => {
                return Err(error(
                    "Release signature verification failed. The installation was not changed.",
                ));
            }
            (Some(_), Some(url)) => {
                let mut response = agent.get(url).call()?;
                Some(bounded_read(response.body_mut().as_reader(), SIGNATURE_LIMIT)?)
            }
        };
        let manifest = verified_manifest(trust, &release.version, bytes, signature.as_deref())?;
        let expected = checksum(&manifest, &release.asset)?;
        let asset = work.path().join(&release.asset);
        download(agent, release, &asset, &expected, &mut progress)?;
        let candidate = crate::install::extract(installation, &asset, work.path(), &release.version)?;
        // The archive is no longer needed, unless an installer consumes the asset itself.
        if candidate != asset {
            fs::remove_file(asset)?;
        }
        Ok(Prepared {
            installation: installation.clone(),
            work,
            candidate,
            sha256: expected,
        })
    }
}

pub(crate) fn download(
    agent: &ureq::Agent,
    release: &Release,
    target: &Path,
    expected: &str,
    progress: &mut impl FnMut(u64, u64) -> bool,
) -> Result<(), Error> {
    if !progress(0, release.size) {
        return Err(error("Update download was cancelled."));
    }
    let mut response = agent.get(&release.url).call()?;
    let mut reader = response.body_mut().as_reader();
    let mut file = File::create(target)?;
    let mut hash = Sha256::new();
    let mut count = 0;
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        count += read as u64;
        if count > release.size {
            return Err(error("Downloaded release exceeds its declared size."));
        }
        file.write_all(&buffer[..read])?;
        hash.update(&buffer[..read]);
        if !progress(count, release.size) {
            return Err(error("Update download was cancelled."));
        }
    }
    file.sync_all()?;
    let actual = hash.finalize();
    let matches = actual
        .iter()
        .enumerate()
        .all(|(index, byte)| u8::from_str_radix(&expected[index * 2..index * 2 + 2], 16).ok() == Some(*byte));
    if count != release.size || !matches {
        return Err(error(
            "Downloaded release failed SHA-256 verification. The installation was not changed.",
        ));
    }
    Ok(())
}

#[cfg(test)]
#[path = "download_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "github_download_tests.rs"]
mod github_tests;
