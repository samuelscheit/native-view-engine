plugins {
    id("com.android.library") version "8.8.2"
    id("org.jetbrains.kotlin.android") version "2.1.10"
    id("org.jetbrains.kotlin.plugin.compose") version "2.1.10"
}

android {
    namespace = "dev.nve.compose"
    compileSdk = 36

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
    // Compose UI tests of `NveSurface` under Robolectric.
    testImplementation(composeBom)
    testImplementation("androidx.compose.ui:ui-test-junit4")
    debugImplementation("androidx.compose.ui:ui-test-manifest")
}

val nveRepoRoot = rootProject.projectDir.resolve("../..").canonicalFile
// `dev.nve.host`: the static Java host runtime plus its generated ABI / table data.
val nveHostJavaDir = nveRepoRoot.resolve("packages/core/host-java/src/main/java")
val nveGeneratedJavaFiles = listOf(
    nveHostJavaDir.resolve("dev/nve/host/generated/NveAbi.java"),
    nveHostJavaDir.resolve("dev/nve/host/generated/NveTables.java"),
    nveHostJavaDir.resolve("dev/nve/host/generated/NveRuntimeAbi.java"),
    nveHostJavaDir.resolve("dev/nve/host/generated/NveModuleTables.java"),
    nveHostJavaDir.resolve("dev/nve/host/generated/NveJniFrame.java"),
)

android.sourceSets.getByName("main").java.srcDir(nveHostJavaDir)
// The zero-Gradle Android View host (`hosts/android-shell/android-app`) is unit-tested here under
// Robolectric (`dev.nve.android` renderer suites): it has no Gradle build of its own.
android.sourceSets.getByName("test").java.srcDir(nveRepoRoot.resolve("hosts/android-shell/android-app/src"))

// Real Kotlin compile & execution verification of `nve-codegen` `export_rn` (Android) +
// `export_compose` generated sources (written inside `build/`, never committed).
val nveExportSpecDir = projectDir.resolve("src/test/resources/export-specs")
val nveExportKotlinOutDir = layout.buildDirectory.dir("generated/nve-export-test")
android.sourceSets.getByName("test").kotlin.srcDir(nveExportKotlinOutDir)

val generateExportKotlinFixtures by tasks.registering {
    group = "codegen"
    description = "Emit real `export_rn` (Android) and `export_compose` .generated.kt test sources via nve-cli."
    inputs.dir(nveExportSpecDir)
    inputs.dir(nveRepoRoot.resolve("packages/core/codegen/src"))
    inputs.dir(nveRepoRoot.resolve("packages/core/codegen/templates"))
    inputs.file(nveRepoRoot.resolve("Cargo.lock"))
    outputs.dir(nveExportKotlinOutDir)
    doLast {
        val outRoot = nveExportKotlinOutDir.get().asFile
        outRoot.deleteRecursively()
        outRoot.mkdirs()
        val specs = nveExportSpecDir.listFiles { f -> f.isFile && f.extension == "json" }?.sortedBy { it.name }.orEmpty()
        check(specs.isNotEmpty()) { "No export spec JSON fixtures found in $nveExportSpecDir" }

        // Build `nve` once and invoke the compiled binary directly for each spec fixture.
        val buildProc = ProcessBuilder("cargo", "build", "--locked", "-q", "-p", "nve-cli", "--bin", "nve")
            .directory(nveRepoRoot)
            .redirectErrorStream(true)
            .start()
        val buildOut = buildProc.inputStream.bufferedReader().readText()
        val buildRc = buildProc.waitFor()
        check(buildRc == 0) { "cargo build -p nve-cli failed ($buildRc):\n$buildOut" }

        val targetDir = System.getenv("CARGO_TARGET_DIR")?.takeIf { it.isNotBlank() }?.let(::File)
            ?: nveRepoRoot.resolve("target")
        val exeName = if (System.getProperty("os.name").lowercase().contains("win")) "nve.exe" else "nve"
        val nveBin = targetDir.resolve("debug").resolve(exeName)
        check(nveBin.isFile) { "Built nve-cli binary not found at $nveBin" }

        for (spec in specs) {
            val outDir = outRoot.resolve(spec.nameWithoutExtension)
            val proc = ProcessBuilder(
                nveBin.absolutePath,
                "codegen", "export", spec.absolutePath, "--target", "rn,compose", "--out", outDir.absolutePath,
            )
                .directory(nveRepoRoot)
                .redirectErrorStream(true)
                .start()
            val output = proc.inputStream.bufferedReader().readText()
            val rc = proc.waitFor()
            check(rc == 0) { "nve codegen export failed ($rc) for ${spec.name}:\n$output" }
        }
    }
}

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
        (it.name.startsWith("compile") && (it.name.endsWith("JavaWithJavac") || it.name.endsWith("Kotlin")))
}.configureEach {
    dependsOn(generateNveAbi)
}

tasks.matching {
    it.name.startsWith("compile") && it.name.endsWith("UnitTestKotlin")
}.configureEach {
    dependsOn(generateExportKotlinFixtures)
}
