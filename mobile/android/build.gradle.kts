import com.diffplug.gradle.spotless.SpotlessExtension
import io.gitlab.arturbosch.detekt.extensions.DetektExtension
import kotlinx.kover.gradle.plugin.dsl.CoverageUnit
import kotlinx.kover.gradle.plugin.dsl.KoverProjectExtension
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
    alias(libs.plugins.kover)
}

dependencies {
    kover(project(":core"))
    kover(project(":app"))
}

// A custom variant shared by the root and modules merges only Android's debug JVM tests.
kover {
    currentProject {
        createVariant("debugJvm") {}
    }
    reports {
        variant("debugJvm") {
            html { onCheck = true }
            xml { onCheck = true }
        }
    }
}

// Apply identical filters to module reports and the root aggregate. Only generated code is excluded;
// screens, previews, activities, lambdas and coroutine bodies remain in the denominator.
// Compose's ComposableSingletons classes hold handwritten lambda bodies, so they must remain included.
allprojects {
    plugins.withId("org.jetbrains.kotlinx.kover") {
        extensions.configure<KoverProjectExtension> {
            reports {
                // Kover defaults to checking all variants. CI's reports and floors select debug explicitly below.
                total {
                    verify { onCheck = false }
                }
                filters {
                    excludes {
                        packages("com.nickalie.ngterm.core.ffi")
                        classes(
                            "*.BuildConfig",
                            "*.R",
                            "*.R\$*",
                            "*\$\$serializer",
                        )
                    }
                }
            }
        }
    }
}

// Whole-percent floors of the debug JVM baseline. Raise these toward 90; never lower an existing floor.
// :core has only generated production bindings today, so its measured denominator is empty.
val coverageFloors =
    mapOf(
        "core" to mapOf(CoverageUnit.LINE to 0, CoverageUnit.INSTRUCTION to 0, CoverageUnit.BRANCH to 0),
        "app" to mapOf(CoverageUnit.LINE to 98, CoverageUnit.INSTRUCTION to 97, CoverageUnit.BRANCH to 79),
    )

subprojects {
    val moduleCoverageFloors = coverageFloors.getValue(name)
    plugins.withId("org.jetbrains.kotlinx.kover") {
        extensions.configure<KoverProjectExtension> {
            currentProject {
                createVariant("debugJvm") {
                    add("debug")
                }
            }
            reports {
                variant("debug") {
                    html { onCheck = true }
                    xml { onCheck = true }
                    verify {
                        onCheck = true
                        rule("JVM coverage floor") {
                            moduleCoverageFloors.forEach { (unit, floor) -> minBound(floor, unit) }
                        }
                    }
                }
            }
        }
    }
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
