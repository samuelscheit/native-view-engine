plugins {
    id("com.android.library") version "8.8.2"
    id("org.jetbrains.kotlin.android") version "2.1.10"
    id("org.jetbrains.kotlin.plugin.compose") version "2.1.10"
}

android {
    namespace = "dev.nve.compose"
    compileSdk = 35

    defaultConfig {
        minSdk = 24
        consumerProguardFiles("consumer-rules.pro")
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    kotlinOptions {
        jvmTarget = "17"
    }

    buildFeatures {
        compose = true
        buildConfig = false
    }

    testOptions {
        unitTests {
            isReturnDefaultValues = true
            isIncludeAndroidResources = true
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
    val composeBom = platform("androidx.compose:compose-bom:2025.02.00")
    implementation(composeBom)
    implementation("androidx.compose.runtime:runtime")
    implementation("androidx.compose.ui:ui")
    implementation("androidx.compose.foundation:foundation")
    implementation("androidx.activity:activity-compose:1.10.0")
    implementation("androidx.lifecycle:lifecycle-runtime-compose:2.8.7")

    // JUnit 5 (pure-JVM binary codec, scene buffer, state & controller tests)
    testImplementation("org.junit.jupiter:junit-jupiter:5.11.4")
    testRuntimeOnly("org.junit.platform:junit-platform-launcher:1.11.4")

    // JUnit 4 + Vintage Engine + Robolectric (Android Activity & View lifecycle tests)
    testImplementation("junit:junit:4.13.2")
    testRuntimeOnly("org.junit.vintage:junit-vintage-engine:5.11.4")
    testImplementation("org.robolectric:robolectric:4.14.1")
}

val nveRepoRoot = rootProject.projectDir.resolve("../..").canonicalFile
// `dev.nve.host`: the static Java host runtime plus its generated ABI / table data.
val nveHostJavaDir = nveRepoRoot.resolve("packages/core/host-java/src/main/java")
val nveGeneratedJavaFiles = listOf(
    nveHostJavaDir.resolve("dev/nve/host/generated/NveAbi.java"),
    nveHostJavaDir.resolve("dev/nve/host/generated/NveTables.java"),
)

android.sourceSets.getByName("main").java.srcDir(nveHostJavaDir)

val generateNveAbi by tasks.registering(Exec::class) {
    group = "codegen"
    description = "Generate the NVE host ABI / table Java bindings via nve-cli when missing."
    workingDir = nveRepoRoot
    commandLine("cargo", "run", "--locked", "-p", "nve-cli", "--", "codegen", "abi")
    onlyIf {
        !project.hasProperty("nveSkipCodegen") && nveGeneratedJavaFiles.any { !it.isFile }
    }
}

tasks.matching {
    it.name == "preBuild" ||
        it.name == "compileJava" ||
        (it.name.startsWith("compile") && it.name.endsWith("JavaWithJavac"))
}.configureEach {
    dependsOn(generateNveAbi)
}
