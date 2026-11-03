// The engine's wire as Kotlin types (packages/shared is the source of truth). Plain JVM: no Android in here
plugins {
  alias(libs.plugins.kotlin.jvm)
  alias(libs.plugins.kotlin.serialization)
}

kotlin {
  jvmToolchain(21)
  compilerOptions { jvmTarget.set(org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17) }
}
java { targetCompatibility = JavaVersion.VERSION_17; sourceCompatibility = JavaVersion.VERSION_17 }

dependencies {
  api(libs.serialization.json)
  testImplementation(libs.kotlin.test)
}

// The contract tests read the repository (packages/shared, test/fixtures)
tasks.test { systemProperty("acpira.root", rootDir.resolve("../..").canonicalPath) }
