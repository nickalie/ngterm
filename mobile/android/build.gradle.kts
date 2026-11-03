import com.diffplug.gradle.spotless.SpotlessExtension
import io.gitlab.arturbosch.detekt.extensions.DetektExtension

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
}
