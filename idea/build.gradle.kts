import org.jetbrains.intellij.platform.gradle.TestFrameworkType

plugins {
    id("java")
    id("org.jetbrains.kotlin.jvm")
    id("org.jetbrains.intellij.platform")
}

group = "com.github.hotic.acpira"
version = providers.gradleProperty("pluginVersion").getOrElse("0.0.1")

kotlin {
    jvmToolchain(21)
}

repositories {
    mavenCentral()
    intellijPlatform {
        defaultRepositories()
    }
}

dependencies {
    intellijPlatform {
        // IntelliJ IDEA is one unified distribution since 2025.3 (no more IC / IU). The installer form is required:
        // the SDK archive (useInstaller = false) ships without the JCEF native components
        intellijIdea(providers.gradleProperty("platformVersion"))
        // Optional at runtime (plugin.xml), needed at compile time for runInTerminal
        bundledPlugin("org.jetbrains.plugins.terminal")
        testFramework(TestFrameworkType.Platform)
    }
    testImplementation("junit:junit:4.13.2")
}

// The webview bundle and the Node sidecar come from the repository build one level up (`pnpm build`): main.{js,css} go into the jar as
// resources the https://acpira.local handler serves, host-server.cjs is packaged next to the plugin so Node can run it as a file
val webviewDist: File = file("../dist/webview")
val sidecarBundle: File = file("../dist/host-server.cjs")

// A task action may only capture plain values (configuration cache): copy the files into locals first
fun requireBuilt(file: File, what: String): Action<Task> {
    val f = file
    return Action { check(f.isFile) { "$what is missing ($f): run `pnpm build` in the repository root first" } }
}

tasks {
    processResources {
        doFirst(requireBuilt(webviewDist.resolve("main.js"), "the webview bundle"))
        from(webviewDist) { into("webview") }
    }
    prepareSandbox {
        doFirst(requireBuilt(sidecarBundle, "the sidecar bundle"))
        from(sidecarBundle) { into(intellijPlatform.projectName.map { "$it/sidecar" }) }
    }
    runIde {
        // A sandbox launched from the terminal opens the project given as -PrunIdeProject (or none); trusting it up front keeps startup
        // activities from waiting behind the trust dialog. -PautoOpen shows the tool window at once, -PjcefDebug exposes CDP on 9222
        systemProperty("idea.trust.all.projects", "true")
        providers.gradleProperty("runIdeProject").orNull?.let { args = listOf(it) }
        if (providers.gradleProperty("autoOpen").isPresent) systemProperty("acpira.dev.autoOpen", "true")
        if (providers.gradleProperty("jcefDebug").isPresent) {
            systemProperty("ide.browser.jcef.debug.port", "9222")
            systemProperty("ide.browser.jcef.debug.port.random.enabled", "false")
        }
        // Point the sandbox at a different sidecar build without repackaging: -PhostServer=/abs/path/host-server.cjs
        providers.gradleProperty("hostServer").orNull?.let { environment("ACPIRA_HOST_SERVER", it) }
    }
    verifyPlugin {
        // The verifier caches IDEs and their bundled plugins (gigabytes) under ~/.pluginVerifier; -PpluginVerifierHome (or the same key in
        // ~/.gradle/gradle.properties) moves that to a disk with room
        providers.gradleProperty("pluginVerifierHome").orNull?.let { systemProperty("plugin.verifier.home.dir", it) }
    }
}

intellijPlatform {
    pluginConfiguration {
        id = "com.github.hotic.acpira"
        name = "Acpira"
        ideaVersion {
            sinceBuild = providers.gradleProperty("pluginSinceBuild")
            // No untilBuild: open-ended compatibility (the recommended default since 2024.3)
            untilBuild = provider { null }
        }
    }
    pluginVerification {
        // The IDE releases the Marketplace would check this sinceBuild against (2026.1, 2026.2 and the next EAP at the moment)
        ides {
            recommended()
        }
    }
}
