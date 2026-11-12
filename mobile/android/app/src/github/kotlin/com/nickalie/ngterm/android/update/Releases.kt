package com.nickalie.ngterm.android.update

import org.json.JSONArray
import org.json.JSONException
import org.json.JSONObject

/** GitHub's API for ngterm's releases, newest first (a page outlasts the web engine's prereleases between apps). */
const val RELEASES_API = "https://api.github.com/repos/nickalie/ngterm/releases?per_page=50"

/** Where a release's assets download from: `<base>v<version>/<name>`. */
const val RELEASE_DOWNLOADS = "https://github.com/nickalie/ngterm/releases/download/"

/** A release's page, which [UpdateOffer.notesUrl] falls back to. */
const val RELEASE_PAGES = "https://github.com/nickalie/ngterm/releases/"

/** The most the release list may weigh; the checksum list's bound too. */
const val METADATA_LIMIT = 2L * 1024 * 1024

/** The most a checksum list's minisign signature may weigh: four short lines. */
const val SIGNATURE_LIMIT = 4L * 1024

/** The largest APK the app takes. */
const val APK_LIMIT = 512L * 1024 * 1024

/** A release to update to, with where its APK, its checksum list and that list's signature download from. */
data class ReleaseDownload(
    val version: ReleaseVersion,
    val apk: String,
    val apkUrl: String,
    val sumsUrl: String,
    val signatureUrl: String,
    val offer: UpdateOffer,
)

/**
 * The newest release in [json] (GitHub's release list) that is newer than [installed] and carries a complete Android
 * download: a unique `ngterm-<version>-android.apk`, `SHA256SUMS` and `SHA256SUMS.minisig`, each at its release's own
 * download URL and within its size bound. Drafts are skipped, and prereleases too unless [installed] is one; other
 * tags (the web engine's `web-engine-*` releases) aren't versions and never count.
 *
 * @throws JSONException when [json] isn't a release list.
 */
fun newestRelease(
    json: String,
    installed: ReleaseVersion,
): ReleaseDownload? {
    val releases = JSONArray(json)
    return (0 until releases.length())
        .map { releases.getJSONObject(it) }
        .mapNotNull { release -> release.candidate(installed)?.let { it to release } }
        .sortedByDescending { it.first }
        .firstNotNullOfOrNull { (version, release) -> release.download(version) }
}

private fun JSONObject.candidate(installed: ReleaseVersion): ReleaseVersion? {
    val version = optString("tag_name").removePrefix("v").takeIf { optString("tag_name").startsWith("v") }
    val parsed = version?.let(ReleaseVersion::parse) ?: return null
    val pre = optBoolean("prerelease") || parsed.isPre
    return parsed.takeIf { !optBoolean("draft") && (!pre || installed.isPre) && it > installed }
}

private fun JSONObject.download(version: ReleaseVersion): ReleaseDownload? {
    val assets = optJSONArray("assets") ?: return null
    val listed = (0 until assets.length()).map { assets.getJSONObject(it) }
    val find = { name: String, limit: Long ->
        listed.singleOrNull { it.optString("name") == name }?.takeIf { asset ->
            asset.optString("browser_download_url") == "${RELEASE_DOWNLOADS}v$version/$name" &&
                asset.optLong("size") in 1..limit
        }
    }
    val apk = "ngterm-$version-android.apk"
    val apkAsset = find(apk, APK_LIMIT) ?: return null
    val sums = find("SHA256SUMS", METADATA_LIMIT) ?: return null
    val signature = find("SHA256SUMS.minisig", SIGNATURE_LIMIT) ?: return null
    val notes = optString("html_url").takeIf { it.startsWith(RELEASE_PAGES) } ?: "${RELEASE_PAGES}tag/v$version"
    return ReleaseDownload(
        version = version,
        apk = apk,
        apkUrl = apkAsset.getString("browser_download_url"),
        sumsUrl = sums.getString("browser_download_url"),
        signatureUrl = signature.getString("browser_download_url"),
        offer = UpdateOffer(version.toString(), apkAsset.getLong("size"), notes),
    )
}
