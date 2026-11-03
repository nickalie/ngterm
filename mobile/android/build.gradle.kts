import com.diffplug.gradle.spotless.SpotlessExtension
import io.gitlab.arturbosch.detekt.extensions.DetektExtension
import org.gradle.api.tasks.testing.logging.TestExceptionFormat
import org.gradle.api.tasks.testing.logging.TestLogEvent
import java.time.Duration

plugins {
    // the root's own `check`, for the Spotless check of the root build scripts
    base
    alias(libs.plugins.android.application) apply false
    alias(libs.plugins.android.library) apply false
    alias(libs.plugins.kotlin.android) apply false
    alias(libs.plugins.kotlin.compose) apply false
    alias(libs.plugins.roborazzi) apply false
    alias(libs.plugins.spotless)
    alias(libs.plugins.detekt) apply false
}

// The lint gate every module shares: ktlint through Spotless for formatting, detekt with
// the Compose rules for static analysis. Both are part of `check`; the Kotlin compiler and Android Lint are set per
// module (their extensions come with the Android plugins).
val ktlintVersion: String = libs.versions.ktlint.get()

spotless {
    kotlinGradle {
        target("*.gradle.kts")
        ktlint(ktlintVersion)
    }
}

subprojects {
    apply(plugin = "com.diffplug.spotless")
    apply(plugin = "io.gitlab.arturbosch.detekt")

    extensions.configure<SpotlessExtension> {
        kotlin {
            target("src/**/*.kt")
            ktlint(ktlintVersion)
        }
        kotlinGradle {
            target("*.gradle.kts")
            ktlint(ktlintVersion)
        }
    }

    extensions.configure<DetektExtension> {
        buildUponDefaultConfig = true
        config.setFrom(rootProject.file("config/detekt/detekt.yml"))
        // every source set the modules have, not only detekt's default main and test
        source.setFrom(
            listOf("main", "test", "testFixtures", "androidTest").map { "src/$it/kotlin" },
        )
        parallel = true
    }

    dependencies {
        add("detektPlugins", rootProject.libs.compose.rules.detekt)
    }

    // The JVM tests take under a minute: a hung one fails its task in minutes rather than at the CI job's limit, and on
    // CI (`CI` set) each test is logged as it starts, so the log names the one that hung.
    val onCi = providers.environmentVariable("CI").isPresent
    tasks.withType<Test>().configureEach {
        timeout.set(Duration.ofMinutes(15))
        testLogging {
            val always = listOf(TestLogEvent.FAILED, TestLogEvent.SKIPPED)
            events = (if (onCi) always + listOf(TestLogEvent.STARTED, TestLogEvent.PASSED) else always).toSet()
            exceptionFormat = TestExceptionFormat.FULL
        }
    }
}

