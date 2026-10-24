// Optional backend module isolates Terminal plugin classes behind the core backend TerminalRunner service.
intellijPlatform {
    projectName = "acpira.backend.terminal"
}

tasks.named<org.jetbrains.intellij.platform.gradle.tasks.ComposedJarTask>("composedJar") {
    archiveFileName = "acpira.backend.terminal.jar"
}

dependencies {
    intellijPlatform {
        intellijIdea(providers.gradleProperty("platformVersion"))
        bundledPlugin("org.jetbrains.plugins.terminal")
    }

    compileOnly(project(":backend"))
}
