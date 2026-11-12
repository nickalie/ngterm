package com.nickalie.ngterm.android.update

import androidx.test.ext.junit.runners.AndroidJUnit4
import org.json.JSONException
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

/** Release versions and picking the release to update to from GitHub's list (org.json is Robolectric's Android's). */
@RunWith(AndroidJUnit4::class)
class ReleasesTest {
    private val installed = ReleaseVersion.parse("0.2.3")!!

    private fun newest(
        vararg releases: String,
        from: ReleaseVersion = installed,
    ) = newestRelease("[${releases.joinToString(",")}]", from)

    @Test
    fun versionsOrderAsSemanticVersioningSays() {
        val ordered =
            listOf(
                "0.9.9",
                "1.0.0-alpha",
                "1.0.0-alpha.1",
                "1.0.0-alpha.beta",
                "1.0.0-beta",
                "1.0.0-beta.2",
                "1.0.0-beta.11",
                "1.0.0-rc.1",
                "1.0.0",
                "1.0.1",
                "1.1.0",
                "2.0.0",
            ).map { ReleaseVersion.parse(it)!! }
        assertEquals(ordered, ordered.shuffled().sorted())
        assertEquals(0, ReleaseVersion.parse("1.0.0+build.5")!!.compareTo(ReleaseVersion.parse("1.0.0")!!))
        assertEquals("1.0.0-rc.1", ReleaseVersion.parse("1.0.0-rc.1").toString())
        assertTrue(ReleaseVersion.parse("1.0.0-rc.1")!!.isPre)
        for (bad in listOf("", "1.0", "v1.0.0", "01.0.0", "1.0.0-", "1.0.0-a..b", "web-engine-abc", "1.0.0 ")) {
            assertNull(bad, ReleaseVersion.parse(bad))
        }
    }

    @Test
    fun theNewestCompleteStableReleaseAboveTheInstalledOneIsPicked() {
        val picked = newest(releaseJson("0.2.3"), releaseJson("0.3.0"), releaseJson("0.2.9"))!!
        assertEquals(ReleaseVersion.parse("0.3.0"), picked.version)
        assertEquals("ngterm-0.3.0-android.apk", picked.apk)
        assertEquals("${RELEASE_DOWNLOADS}v0.3.0/ngterm-0.3.0-android.apk", picked.apkUrl)
        assertEquals("${RELEASE_DOWNLOADS}v0.3.0/SHA256SUMS", picked.sumsUrl)
        assertEquals("${RELEASE_DOWNLOADS}v0.3.0/SHA256SUMS.minisig", picked.signatureUrl)
        assertEquals(UpdateOffer("0.3.0", apkBytes("0.3.0").size.toLong(), "${RELEASE_PAGES}tag/v0.3.0"), picked.offer)
        assertNull(newest(releaseJson("0.2.3"), releaseJson("0.1.0")))
        assertNull(newest())
    }

    @Test
    fun draftsPrereleasesAndOtherTagsArePassedOver() {
        val engine = """{"tag_name":"web-engine-abc","draft":false,"prerelease":true,"assets":[]}"""
        assertNull(
            newest(
                engine,
                releaseJson("0.4.0", draft = true),
                releaseJson("0.4.0-rc.1"),
                releaseJson("0.5.0", prerelease = true),
            ),
        )
        assertEquals("0.3.0", newest(engine, releaseJson("0.4.0-rc.1"), releaseJson("0.3.0"))!!.offer.version)
    }

    @Test
    fun aPrereleaseBuildAlsoTakesNewerPrereleases() {
        val rc = ReleaseVersion.parse("0.4.0-rc.1")!!
        assertEquals(
            "0.4.0-rc.2",
            newest(releaseJson("0.4.0-rc.2", prerelease = true), releaseJson("0.3.0"), from = rc)!!.offer.version,
        )
        assertEquals(
            "0.4.0",
            newest(releaseJson("0.4.0-rc.2", prerelease = true), releaseJson("0.4.0"), from = rc)!!.offer.version,
        )
    }

    @Test
    fun aReleaseWithoutACompleteAndroidDownloadIsSkippedForTheNextOne() {
        val apk = "ngterm-0.4.0-android.apk" to 10L
        val sums = "SHA256SUMS" to 10L
        val signature = "SHA256SUMS.minisig" to 10L
        val incomplete =
            listOf(
                listOf(sums, signature),
                listOf(apk, signature),
                listOf(apk, sums),
                listOf(apk, apk, sums, signature),
                listOf(apk.first to 0L, sums, signature),
                listOf(apk.first to APK_LIMIT + 1, sums, signature),
                listOf(apk, sums.first to METADATA_LIMIT + 1, signature),
                listOf(apk, sums, signature.first to SIGNATURE_LIMIT + 1),
            )
        for (assets in incomplete) {
            assertEquals(
                assets.toString(),
                "0.3.0",
                newest(releaseJson("0.4.0", assets = assets), releaseJson("0.3.0"))!!.offer.version,
            )
        }
        // an asset served from anywhere but its own release's downloads
        val elsewhere = releaseJson("0.4.0", base = "https://example.com/")
        assertEquals("0.3.0", newest(elsewhere, releaseJson("0.3.0"))!!.offer.version)
        assertNull(newest("""{"tag_name":"v0.4.0"}"""))
    }

    @Test
    fun aReleasePageElsewhereFallsBackToItsGitHubPage() {
        val json = releaseJson("0.3.0").replace("${RELEASE_PAGES}tag/v0.3.0", "https://example.com/notes")
        assertEquals("${RELEASE_PAGES}tag/v0.3.0", newest(json)!!.offer.notesUrl)
    }

    @Test
    fun somethingOtherThanAReleaseListIsAnError() {
        assertThrows(JSONException::class.java) { newestRelease("{}", installed) }
        assertThrows(JSONException::class.java) { newestRelease("[1]", installed) }
    }
}
