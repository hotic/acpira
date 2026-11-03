// The envelope client, transports and session patches. Plain JVM: no Android in here
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
  api(project(":model"))
  api(libs.coroutines.core)
  implementation(libs.okhttp)
  testImplementation(libs.kotlin.test)
  testImplementation(libs.coroutines.test)
}

tasks.test { systemProperty("acpira.root", rootDir.resolve("../..").canonicalPath) }
