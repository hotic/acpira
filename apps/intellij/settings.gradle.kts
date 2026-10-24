import org.jetbrains.intellij.platform.gradle.extensions.intellijPlatform

rootProject.name = "acpira"

pluginManagement {
    repositories {
        mavenCentral()
        gradlePluginPortal()
        maven("https://packages.jetbrains.team/maven/p/ij/intellij-dependencies/")
    }
    plugins {
        // IntelliJ Platform 2026.1 ships Kotlin 2.3 metadata; the compiler must be on the same line.
        // The `rpc` compiler plugin is bound to an exact Kotlin version: 2026.1 -> rpc 2.3.20-RC2-0.1 on Kotlin 2.3.20
        id("rpc") version "2.3.20-RC2-0.1"
        id("org.jetbrains.kotlin.jvm") version "2.3.20"
        id("org.jetbrains.kotlin.plugin.serialization") version "2.3.20"
    }
}

plugins {
    id("org.jetbrains.intellij.platform.settings") version "2.18.1"
}

@Suppress("UnstableApiUsage")
dependencyResolutionManagement {
    repositories {
        mavenCentral()
        intellijPlatform {
            defaultRepositories()
        }
    }
}

include("shared", "frontend", "backend", "backend-terminal", "browser-legacy", "browser-modular")
