// The Compose app. Fonts, dictionaries and demo fixtures are the iOS app's own files, synced in at build time so both
// apps (and the webview the dictionaries come from) stay one source
plugins {
  alias(libs.plugins.android.application)
  alias(libs.plugins.kotlin.compose)
  alias(libs.plugins.kotlin.serialization)
}

val ios = rootDir.resolve("../ios/Packages/AcpiraKit/Sources")
val sharedAssets = layout.buildDirectory.dir("generated/acpira-assets")
// Paired remote access: acpira-mobile's library and Kotlin bindings from scripts/build-net.sh (git-ignored). Without them
// the app builds with `src/nonet`, which offers no pairing service, as the iOS package leaves AcpiraRemote out
val net = rootDir.resolve("net")
val withNet = net.resolve("stamp").exists()

val syncSharedAssets by tasks.registering(Sync::class) {
  into(sharedAssets)
  from(ios.resolve("AcpiraUI/Resources/i18n")) { into("i18n") }
  from(ios.resolve("AcpiraDesign/Fonts")) { include("InterVariable*.ttf", "KaTeX_*.ttf"); into("fonts") }
  // Transcript text: KaTeX's metric tables (read from the generated Swift source), shiki's grammars and themes, and
  // the browser mermaid for the diagrams the native flowchart does not read
  from(ios.resolve("AcpiraDesign")) { include("KaTeXMetrics.generated.swift"); into("katex") }
  from(ios.resolve("AcpiraUI/Resources/shiki")) { into("shiki") }
  from(ios.resolve("AcpiraUI/Resources/web")) { include("mermaid.min.js"); into("web") }
  from(ios.resolve("AcpiraFixtures/synthetic")) { into("fixtures") }
  // Tailwind's browser build for HTML widgets (ui/WidgetView.kt)
  from(ios.resolve("AcpiraUI/Resources/web")) { include("tailwind.js"); into("web") }
}

android {
  namespace = "cc.hotic.acpira"
  compileSdk = 37

  defaultConfig {
    applicationId = "cc.hotic.acpira"
    minSdk = 29
    targetSdk = 37
    versionCode = 1
    versionName = "0.1.0"
  }

  buildTypes {
    release {
      isMinifyEnabled = false
    }
    // Timing builds (touch-sim's BENCH=1): release code, not debuggable, so ART compiles it as it would the store
    // build; debug-signed so it installs over a debug build; profileable from the shell (src/bench/AndroidManifest.xml)
    create("bench") {
      initWith(getByName("release"))
      signingConfig = signingConfigs.getByName("debug")
      matchingFallbacks += "release"
    }
  }
  compileOptions {
    sourceCompatibility = JavaVersion.VERSION_17
    targetCompatibility = JavaVersion.VERSION_17
  }
  buildFeatures { compose = true }
  sourceSets {
    getByName("main") {
      assets.srcDir(sharedAssets.get().asFile)
      if (withNet) {
        kotlin.srcDirs("src/net/kotlin", net.resolve("kotlin"))
        jniLibs.srcDir(net.resolve("jniLibs"))
      } else {
        kotlin.srcDir("src/nonet/kotlin")
      }
    }
  }
  packaging { resources { excludes += "/META-INF/{AL2.0,LGPL2.1}" } }
}

kotlin { jvmToolchain(21) }

tasks.named("preBuild") { dependsOn(syncSharedAssets) }

dependencies {
  implementation(project(":sync"))
  implementation(platform(libs.compose.bom))
  implementation(libs.compose.ui)
  implementation(libs.compose.foundation)
  implementation(libs.compose.material3)
  implementation(libs.compose.ui.tooling.preview)
  implementation(libs.activity.compose)
  implementation(libs.coroutines.android)
  if (withNet) implementation(libs.jna) { artifact { type = "aar" } }
  implementation(libs.camera.camera2)
  implementation(libs.camera.lifecycle)
  implementation(libs.camera.view)
  implementation(libs.zxing.core)
  implementation(libs.javascriptengine)
  implementation(libs.termlib)
  testImplementation(libs.junit)
  testImplementation(libs.kotlin.test)
  debugImplementation(libs.compose.ui.tooling)
}
