// Zero-dependency NVE host runtime for the JVM and Android (`dev.nve.host`, Java 11 bytecode).
// Consumers: `includeBuild("../../packages/core/host-java")` + `implementation("dev.nve:nve-host-java")`.
plugins {
    `java-library`
}

group = "dev.nve"
version = "0.1.0"

tasks.withType<JavaCompile>().configureEach {
    options.release.set(11)
    options.compilerArgs.addAll(listOf("-Xlint:all", "-Werror"))
}

// The suite is a plain `main` (no test framework download), so it also runs offline.
val hostTests by tasks.registering(JavaExec::class) {
    description = "Runs the host-java golden-vector, codec, and table/scene suites."
    group = "verification"
    classpath = sourceSets["test"].runtimeClasspath
    mainClass.set("dev.nve.host.HostTestMain")
}

// `gradle test` / `gradle check` run the suite; the framework-based `Test` task has nothing to run.
tasks.test {
    enabled = false
    dependsOn(hostTests)
}
