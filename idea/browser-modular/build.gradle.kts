intellijPlatform { projectName = "acpira.browser.modular" }
tasks.named<org.jetbrains.intellij.platform.gradle.tasks.ComposedJarTask>("composedJar") {
    archiveFileName = "acpira.browser.modular.jar"
}

// Both adapters compile the same browser implementation against the supported API baseline.
kotlin.sourceSets.main { kotlin.srcDir("../browser-runtime/src/main/kotlin") }
dependencies {
    intellijPlatform {
        intellijIdea(providers.gradleProperty("platformVersion"))
        bundledModule("intellij.platform.frontend")
        testFramework(org.jetbrains.intellij.platform.gradle.TestFrameworkType.Platform)
    }
    compileOnly(project(":frontend"))
    compileOnly(project(":shared"))
    compileOnly("org.jetbrains.kotlinx:kotlinx-serialization-core-jvm:1.9.0")
    compileOnly("org.jetbrains.kotlinx:kotlinx-serialization-json-jvm:1.9.0")
    testImplementation(project(":frontend"))
    testImplementation(project(":shared"))
    testImplementation("junit:junit:4.13.2")
}
