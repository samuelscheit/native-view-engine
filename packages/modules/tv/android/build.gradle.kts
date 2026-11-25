plugins {
    id("com.android.library") version "8.8.2"
    id("org.jetbrains.kotlin.android") version "2.1.10"
}

android {
    namespace = "dev.nve.tv"
    compileSdk = 35

    defaultConfig {
        minSdk = 24
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    kotlinOptions {
        jvmTarget = "17"
    }

    buildFeatures {
        buildConfig = false
    }

    testOptions {
        unitTests {
            isReturnDefaultValues = true
            all { test ->
                test.useJUnitPlatform()
                test.testLogging {
                    events("passed", "skipped", "failed")
                    showStandardStreams = false
                }
            }
        }
    }
}

dependencies {
    testImplementation("org.junit.jupiter:junit-jupiter:5.11.4")
    testRuntimeOnly("org.junit.platform:junit-platform-launcher:1.11.4")
}
