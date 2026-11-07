// The ngterm Android app: Compose over the mobile core's binding (:core).
plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.compose)
    alias(libs.plugins.roborazzi)
}

android {
    namespace = "com.nickalie.ngterm.android"
    compileSdk =
        libs.versions.compileSdk
            .get()
            .toInt()

    defaultConfig {
        applicationId = "com.nickalie.ngterm.android"
        minSdk =
            libs.versions.minSdk
                .get()
                .toInt()
        targetSdk =
            libs.versions.targetSdk
                .get()
                .toInt()
        // a release build gets both from release.yml's version job (scripts/release-version.py); local builds keep these
        versionCode = providers.gradleProperty("ngterm.versionCode").orNull?.toInt() ?: 1
        versionName = providers.gradleProperty("ngterm.versionName").orNull ?: "0.1.0"
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
        ndk {
            // the ABIs scripts/build-android.sh builds the mobile core for; JNA's other ABIs would only crash
            abiFilters += setOf("arm64-v8a", "x86_64")
        }
    }

    buildFeatures {
        compose = true
    }

    testOptions {
        unitTests {
            // Robolectric (Roborazzi's screenshots) renders with the app's resources
            isIncludeAndroidResources = true
        }
    }

    lint {
        warningsAsErrors = true
        abortOnError = true
        checkDependencies = true
        // Checks whose verdict changes with the calendar, not the code: a release elsewhere (a newer library, AGP or
        // Android) would break a pinned build. Upgrades are deliberate edits of gradle/libs.versions.toml.
        disable += setOf("OldTargetApi", "NewerVersionAvailable", "GradleDependency", "AndroidGradlePluginVersion")
    }
}

kotlin {
    compilerOptions {
        allWarningsAsErrors.set(true)
        progressiveMode.set(true)
    }
}

// the Rust writer's golden row buffer, which RowBufferReaderTest decodes
val rowsGolden = rootProject.file("../../crates/ngterm-mobile/tests/fixtures/rows-golden.bin")

tasks.withType<Test>().configureEach {
    // Robolectric's SDK 36 sandbox reaches into java.base's FileDescriptor internals (its FileDescriptorInterceptor)
    jvmArgs("--add-opens=java.base/jdk.internal.access=ALL-UNNAMED")
    inputs.file(rowsGolden).withPropertyName("rowsGolden")
    systemProperty("ngterm.rowsGolden", rowsGolden.absolutePath)
}

roborazzi {
    // the goldens are committed next to the tests that write them
    outputDir.set(file("src/test/screenshots"))
}

dependencies {
    implementation(project(":core"))

    implementation(libs.androidx.core.ktx)
    implementation(libs.androidx.activity.compose)
    implementation(libs.androidx.lifecycle.runtime.compose)
    implementation(libs.androidx.lifecycle.viewmodel.compose)
    implementation(libs.androidx.navigation.compose)
    implementation(libs.androidx.datastore.preferences)
    implementation(platform(libs.compose.bom))
    implementation(libs.compose.ui)
    implementation(libs.compose.ui.tooling.preview)
    implementation(libs.compose.material3)
    implementation(libs.compose.material3.window.sizeclass)
    implementation(libs.compose.material3.adaptive)
    implementation(libs.compose.material3.adaptive.layout)
    implementation(libs.compose.material3.adaptive.navigation)
    implementation(libs.compose.material3.adaptive.navigation.suite)
    implementation(libs.markdown.renderer.m3)
    debugImplementation(libs.compose.ui.tooling)
    debugImplementation(libs.compose.ui.test.manifest)

    testImplementation(testFixtures(project(":core")))
    testImplementation(libs.junit)
    testImplementation(libs.androidx.test.ext.junit)
    testImplementation(libs.robolectric)
    testImplementation(libs.roborazzi)
    testImplementation(libs.roborazzi.compose)
    testImplementation(libs.roborazzi.junit.rule)
    testImplementation(platform(libs.compose.bom))
    testImplementation(libs.compose.ui.test.junit4)

    androidTestImplementation(testFixtures(project(":core")))
    androidTestImplementation(libs.androidx.test.core)
    androidTestImplementation(libs.androidx.test.runner)
    androidTestImplementation(libs.androidx.test.ext.junit)
    androidTestImplementation(platform(libs.compose.bom))
    androidTestImplementation(libs.compose.ui.test.junit4)
}
