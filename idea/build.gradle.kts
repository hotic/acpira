import org.jetbrains.kotlin.gradle.dsl.KotlinJvmProjectExtension
import org.jetbrains.intellij.platform.gradle.tasks.ComposedJarTask
import org.jetbrains.intellij.platform.gradle.tasks.PrepareSandboxTask
import org.jetbrains.intellij.platform.gradle.tasks.RunIdeTask
import org.jetbrains.intellij.platform.gradle.tasks.aware.SplitModeAware
import java.security.MessageDigest

plugins {
    id("java")
    id("org.jetbrains.kotlin.jvm")
    id("rpc") apply false
    id("org.jetbrains.kotlin.plugin.serialization") apply false
    id("org.jetbrains.intellij.platform")
}

group = "com.github.hotic.acpira"
// One version for both shells: the plugin carries the extension's package.json version (the release workflow tags that)
val packageJson = providers.fileContents(layout.projectDirectory.file("../package.json")).asText
version = packageJson.map { Regex("\"version\"\\s*:\\s*\"([^\"]+)\"").find(it)?.groupValues?.get(1) ?: error("no version in package.json") }.get()

kotlin {
    jvmToolchain(21)
}

subprojects {
    apply(plugin = "org.jetbrains.intellij.platform.module")
    apply(plugin = "org.jetbrains.kotlin.jvm")
    apply(plugin = "org.jetbrains.kotlin.plugin.serialization")
    apply(plugin = "rpc")
    extensions.configure<KotlinJvmProjectExtension> { jvmToolchain(21) }
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
        pluginModule(implementation(project(":shared")))
        pluginModule(implementation(project(":frontend")))
        pluginModule(implementation(project(":browser-legacy")))
        pluginModule(implementation(project(":browser-modular")))
        pluginModule(implementation(project(":backend")))
        pluginModule(implementation(project(":backend-terminal")))
    }
}

// The Node sidecar comes from the repository build one level up (`pnpm build`) and is packaged next to the plugin so Node can run it as a file
val sidecarBundle: File = file("../dist/host-server.cjs")

// A task action may only capture plain values (configuration cache): copy the files into locals first
fun requireBuilt(file: File, what: String, command: String = "pnpm build"): Action<Task> {
    val f = file
    return Action { check(f.isFile) { "$what is missing ($f): run `$command` in the repository root first" } }
}

// The Rust sidecar, one binary per platform, built into ../dist/sidecar/<os>-<arch>/ by `pnpm build:sidecar` (scripts/build-sidecar.mjs;
// CI builds all six). Packaged as <plugin>/sidecar/bin/<os>-<arch>/acpira[.exe], where SidecarLocator looks for it
val sidecarPlatforms = listOf("mac_arm64", "mac_x86_64", "linux_arm64", "linux_x86_64", "windows_arm64", "windows_x86_64")
fun sidecarDir(variant: String): File = file("../dist/sidecar/${variant.replaceFirst('_', '-')}")
fun sidecarBinary(variant: String): File = sidecarDir(variant).resolve(if (variant.startsWith("windows")) "acpira.exe" else "acpira")

val hostVariant: String = run {
    val os = System.getProperty("os.name").lowercase()
    val arch = System.getProperty("os.arch").lowercase()
    (if (os.contains("mac")) "mac" else if (os.contains("win")) "windows" else "linux") + "_" + (if (arch == "aarch64" || arch == "arm64") "arm64" else "x86_64")
}

// A sandbox launched from the terminal opens the project given as -PrunIdeProject (or none); trusting it up front keeps startup
// activities from waiting behind the trust dialog. -PautoOpen shows the tool window at once, -PjcefDebug exposes CDP on 9222,
// -PhostServer=/abs/path/host-server.cjs points the sandbox at a different sidecar build without repackaging,
// -PsidecarBin=/abs/path/acpira runs the Rust sidecar binary instead
fun RunIdeTask.acpiraDevIde() {
    systemProperty("idea.trust.all.projects", "true")
    // argumentProviders, not args=: split-mode runIde tasks reject direct arguments (they are routed to the backend process)
    providers.gradleProperty("runIdeProject").orNull?.let { p -> argumentProviders.add { listOf(p) } }
    if (providers.gradleProperty("autoOpen").isPresent) systemProperty("acpira.dev.autoOpen", "true")
    if (providers.gradleProperty("jcefDebug").isPresent) {
        systemProperty("ide.browser.jcef.debug.port", "9222")
        systemProperty("ide.browser.jcef.debug.port.random.enabled", "false")
    }
    providers.gradleProperty("hostServer").orNull?.let { environment("ACPIRA_HOST_SERVER", it) }
    providers.gradleProperty("sidecarBin").orNull?.let { environment("ACPIRA_SIDECAR_BIN", it) }
}

tasks {
    named("test") {
        dependsOn(subprojects.map { "${it.path}:test" })
    }
    // Every runIde task has its own sandbox (prepareSandbox_<name>); they all need the sidecar script, and the Rust binary for this
    // machine when `pnpm build:sidecar` has produced it (without one the sandbox runs the Node sidecar, as -PhostServer also forces)
    withType<PrepareSandboxTask>().configureEach {
        doFirst(requireBuilt(sidecarBundle, "the sidecar bundle"))
        from(sidecarBundle) { into(intellijPlatform.projectName.map { "$it/sidecar" }) }
        from(sidecarDir(hostVariant)) {
            into(intellijPlatform.projectName.map { "$it/sidecar/bin/${hostVariant.replaceFirst('_', '-')}" })
            filesMatching("acpira") { permissions { unix("rwxr-xr-x") } }
        }
    }
    buildPlugin {
        // buildPlugin zips the sandbox plugin dir; the binary placed there for runIde belongs to the platform packages only
        exclude("sidecar/bin/**")
    }
    runIde {
        acpiraDevIde()
    }
    verifyPlugin {
        // The verifier caches IDEs and their bundled plugins (gigabytes) under ~/.pluginVerifier; -PpluginVerifierHome (or the same key in
        // ~/.gradle/gradle.properties) moves that to a disk with room
        providers.gradleProperty("pluginVerifierHome").orNull?.let { systemProperty("plugin.verifier.home.dir", it) }
    }
}

// Split-mode development: separate frontend / backend processes locally, plugin installed on BOTH / FRONTEND / BACKEND.
// Registered per target instead of the global intellijPlatform.splitMode so plain `runIde` keeps its monolith behaviour
val runIdeSplitBoth by intellijPlatformTesting.runIde.registering {
    splitMode = true
    pluginInstallationTarget = SplitModeAware.PluginInstallationTarget.BOTH
    task { acpiraDevIde() }
}
val runIdeSplitFrontend by intellijPlatformTesting.runIde.registering {
    splitMode = true
    pluginInstallationTarget = SplitModeAware.PluginInstallationTarget.FRONTEND
    task { acpiraDevIde() }
}
val runIdeSplitBackend by intellijPlatformTesting.runIde.registering {
    splitMode = true
    pluginInstallationTarget = SplitModeAware.PluginInstallationTarget.BACKEND
    task { acpiraDevIde() }
}

// "What's New" on the Marketplace: the CHANGELOG.md section of this version, Keep a Changelog markdown rendered to the HTML subset
// the listing accepts (h3 / ul / p, inline code and links)
val releaseNotes = providers.fileContents(layout.projectDirectory.file("../CHANGELOG.md")).asText.zip(provider { version.toString() }) { text, v ->
    val section = text.split(Regex("(?m)^## ")).firstOrNull { it.startsWith("[$v]") } ?: return@zip ""
    fun inline(s: String) = s.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")
        .replace(Regex("`([^`]+)`"), "<code>$1</code>")
        .replace(Regex("\\[([^]]+)]\\(([^)]+)\\)"), "<a href=\"$2\">$1</a>")
    val out = StringBuilder()
    var inList = false
    for (line in section.lines().drop(1)) {
        val item = line.startsWith("- ")
        if (inList && !item && line.isNotBlank()) { out.append("</ul>\n"); inList = false }
        when {
            line.startsWith("### ") -> out.append("<h3>${inline(line.removePrefix("### "))}</h3>\n")
            item -> { if (!inList) { out.append("<ul>\n"); inList = true }; out.append("<li>${inline(line.removePrefix("- "))}</li>\n") }
            line.isNotBlank() -> out.append("<p>${inline(line)}</p>\n")
        }
    }
    if (inList) out.append("</ul>\n")
    out.toString()
}

intellijPlatform {
    pluginConfiguration {
        id = "com.github.hotic.acpira"
        name = "Acpira"
        changeNotes = releaseNotes
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

// Six per-platform distributions: the root plugin version is suffixed -<os>-<arch>, while the corresponding os / arch plugin
// dependencies live in the backend module descriptor. The plain `buildPlugin` zip carries no binary and runs the Node sidecar from the
// shell PATH
val pluginName = intellijPlatform.projectName
val backendComposedJar = project(":backend").tasks.named<ComposedJarTask>("composedJar").flatMap { it.archiveFile }
val buildPluginVariant = sidecarPlatforms.associateWith { variant ->
    val (os, arch) = variant.split('_', limit = 2)
    val variantRootJar = tasks.register<Jar>("pluginVariantJar_$variant") {
        val composed = tasks.composedJar.flatMap { it.archiveFile }
        from(zipTree(composed)) { exclude("META-INF/plugin.xml") }
        from(zipTree(composed)) {
            include("META-INF/plugin.xml")
            filter { line ->
                if (line.trim().startsWith("<version>")) line.replace("</version>", "-$os-$arch</version>") else line
            }
        }
        archiveFileName = "${pluginName.get()}-$version.jar"
        destinationDirectory = layout.buildDirectory.dir("variant-jars/$variant/root")
    }
    val variantBackendJar = tasks.register<Jar>("pluginVariantBackendJar_$variant") {
        from(zipTree(backendComposedJar)) { exclude("acpira.backend.xml") }
        from(zipTree(backendComposedJar)) {
            include("acpira.backend.xml")
            filter { line ->
                if (line.trim() == "</dependencies>") {
                    "        <plugin id=\"com.intellij.modules.os.$os\"/>\n        <plugin id=\"com.intellij.modules.arch.$arch\"/>\n$line"
                } else line
            }
        }
        archiveFileName = "acpira.backend.jar"
        destinationDirectory = layout.buildDirectory.dir("variant-jars/$variant/backend")
    }
    tasks.register<Zip>("buildPluginVariant_$variant") {
        group = "build"
        description = "Builds the plugin distribution for $os $arch with its sidecar binary"
        doFirst(requireBuilt(sidecarBinary(variant), "the $os-$arch sidecar binary", "pnpm build:sidecar $os-$arch"))
        val base = tasks.buildPlugin.flatMap { it.archiveFile }
        from(zipTree(base)) {
            exclude("*/lib/${pluginName.get()}-*.jar")
            exclude("*/lib/modules/acpira.backend.jar")
        }
        from(variantRootJar) { into(pluginName.map { "$it/lib" }) }
        from(variantBackendJar) { into(pluginName.map { "$it/lib/modules" }) }
        // Zip does not keep source modes; the IDE's installer restores what the entry says, so the binary must be marked here
        from(sidecarDir(variant)) {
            into(pluginName.map { "$it/sidecar/bin/$os-$arch" })
            filesMatching("acpira") { permissions { unix("rwxr-xr-x") } }
        }
        archiveBaseName = pluginName
        archiveClassifier = "$os-$arch"
        destinationDirectory = layout.buildDirectory.dir("distributions")
    }
}
val buildPluginVariants by tasks.registering {
    group = "build"
    description = "Builds all six per-platform plugin distributions"
    dependsOn(buildPluginVariant.values)
}

// Optional content-module constraints do not constrain the whole plugin. Marketplace receives one universal package so
// a macOS client and a Linux backend can install the same version without competing OS-specific updates.
val buildMarketplacePlugin by tasks.registering(Zip::class) {
    group = "build"
    description = "Builds the cross-platform Marketplace package with all six sidecar binaries"
    from(zipTree(tasks.buildPlugin.flatMap { it.archiveFile }))
    for (variant in sidecarPlatforms) {
        doFirst(requireBuilt(sidecarBinary(variant), "the ${variant.replaceFirst('_', '-')} sidecar binary", "pnpm build:sidecar --all"))
        from(sidecarDir(variant)) {
            into(pluginName.map { "$it/sidecar/bin/${variant.replaceFirst('_', '-')}" })
            filesMatching("acpira") { permissions { unix("rwxr-xr-x") } }
        }
    }
    archiveBaseName = pluginName
    archiveClassifier = "universal"
    destinationDirectory = layout.buildDirectory.dir("distributions")
}

// SHA-256 of every distribution zip, next to them, for the release notes
val checksums by tasks.registering {
    group = "build"
    description = "Writes SHA256SUMS for the plugin distributions"
    val dir = layout.buildDirectory.dir("distributions")
    dependsOn(tasks.buildPlugin, buildPluginVariants, buildMarketplacePlugin)
    val prefix = "${pluginName.get()}-$version"
    inputs.files(dir.map { it.asFileTree.matching { include("$prefix*.zip") } })
    outputs.file(dir.map { it.file("SHA256SUMS") })
    doLast {
        val d = dir.get().asFile
        val zips = d.listFiles { f -> f.extension == "zip" && f.name.startsWith(prefix) }!!.sortedBy { it.name }
        d.resolve("SHA256SUMS").writeText(zips.joinToString("") { f ->
            MessageDigest.getInstance("SHA-256").digest(f.readBytes()).joinToString("") { "%02x".format(it) } + "  ${f.name}\n"
        })
    }
}
