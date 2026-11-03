// The Android app: `model` mirrors the engine's wire, `sync` speaks it (both plain JVM, so their tests run without an
// emulator), `app` is the Compose UI. Mirrors apps/ios/Packages/AcpiraKit's AcpiraModels / AcpiraSync / AcpiraUI split.
pluginManagement {
  repositories {
    google()
    mavenCentral()
    gradlePluginPortal()
  }
}

dependencyResolutionManagement {
  repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS)
  repositories {
    google()
    mavenCentral()
  }
}

rootProject.name = "acpira-android"
include(":model", ":sync", ":app")
