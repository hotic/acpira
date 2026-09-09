import org.jetbrains.intellij.platform.gradle.TestFrameworkType
import java.net.URI
import java.security.MessageDigest

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

// One official Node.js archive → `<out>/node/node` (`node.exe` on Windows) + its LICENSE. The digest is the line pinned in
// node-sha256.txt, so a swapped download fails the build instead of shipping. Cached by version + digest
@CacheableTask
abstract class FetchNode @Inject constructor(private val archives: ArchiveOperations, private val fs: FileSystemOperations) : DefaultTask() {
    @get:Input abstract val version: Property<String>
    @get:Input abstract val platform: Property<String>
    @get:Input abstract val sha256: Property<String>
    @get:OutputDirectory abstract val out: DirectoryProperty

    @TaskAction
    fun fetch() {
        val plat = platform.get()
        val ext = if (plat.startsWith("win")) "zip" else "tar.gz"
        val name = "node-v${version.get()}-$plat.$ext"
        val archive = temporaryDir.resolve(name)
        URI("https://nodejs.org/dist/v${version.get()}/$name").toURL().openStream().use { input -> archive.outputStream().use { input.copyTo(it) } }
        val digest = MessageDigest.getInstance("SHA-256").digest(archive.readBytes()).joinToString("") { "%02x".format(it) }
        check(digest == sha256.get()) { "$name: SHA-256 $digest does not match node-sha256.txt (${sha256.get()})" }
        val tree = if (ext == "zip") archives.zipTree(archive) else archives.tarTree(archive)
        val dest = out.get().asFile.also { it.deleteRecursively() }
        fs.copy {
            from(tree) {
                include("*/bin/node", "*/node.exe", "*/LICENSE")
                eachFile { relativePath = RelativePath(true, "node", sourceName) }
                includeEmptyDirs = false
            }
            into(dest)
        }
        check(dest.resolve("node").listFiles()?.any { it.name.startsWith("node") } == true) { "$name: no node executable found in the archive" }
        archive.delete()
    }
}

// nativeVariants naming ↔ nodejs.org naming
val nodePlatforms = mapOf(
    "mac_arm64" to "darwin-arm64", "mac_x86_64" to "darwin-x64",
    "linux_arm64" to "linux-arm64", "linux_x86_64" to "linux-x64",
    "windows_arm64" to "win-arm64", "windows_x86_64" to "win-x64",
)
val nodeVersion = providers.gradleProperty("nodeVersion")
val nodeDigests = providers.fileContents(layout.projectDirectory.file("node-sha256.txt")).asText.map { text ->
    text.lines().filter { it.isNotBlank() && !it.startsWith("#") }.associate { line -> line.substringAfterLast(' ') to line.substringBefore(' ') }
}
val fetchNode = nodePlatforms.mapValues { (variant, plat) ->
    tasks.register<FetchNode>("fetchNode_$variant") {
        group = "build"
        description = "Downloads and verifies the Node.js runtime for $plat"
        version = nodeVersion
        platform = plat
        sha256 = nodeVersion.zip(nodeDigests) { v, d ->
            val ext = if (plat.startsWith("win")) "zip" else "tar.gz"
            d["node-v$v-$plat.$ext"] ?: error("node-sha256.txt has no digest for node-v$v-$plat.$ext")
        }
        out = layout.buildDirectory.dir("node/$plat")
    }
}
val hostVariant: String = run {
    val os = System.getProperty("os.name").lowercase()
    val arch = System.getProperty("os.arch").lowercase()
    (if (os.contains("mac")) "mac" else if (os.contains("win")) "windows" else "linux") + "_" + (if (arch == "aarch64" || arch == "arm64") "arm64" else "x86_64")
}

tasks {
    processResources {
        doFirst(requireBuilt(webviewDist.resolve("main.js"), "the webview bundle"))
        from(webviewDist) { into("webview") }
    }
    prepareSandbox {
        doFirst(requireBuilt(sidecarBundle, "the sidecar bundle"))
        from(sidecarBundle) { into(intellijPlatform.projectName.map { "$it/sidecar" }) }
        // The sandbox runs on the bundled runtime like an installed variant would; -PsystemNode leaves it out to exercise the PATH fallback
        if (!providers.gradleProperty("systemNode").isPresent) from(fetchNode[hostVariant]!!) { into(intellijPlatform.projectName) }
    }
    buildPlugin {
        // buildPlugin zips the sandbox plugin dir; the runtime placed there for runIde belongs to the per-platform variants only
        exclude("node/**")
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

// Six per-platform distributions, the shape the IntelliJ Platform Gradle Plugin's `nativeVariants` will produce once it ships (in its
// [next] changelog at 2.18.1): the buildPlugin zip plus <plugin>/node/node, a version suffixed -<os>-<arch>, and dependencies on the
// os / arch module aliases the platform registers from IdeaPluginOsRequirement / PluginCpuArchRequirement, so an IDE only loads its own.
// The plain `buildPlugin` zip stays runtime-free and falls back to the shell PATH
val pluginName = intellijPlatform.projectName
val buildPluginVariant = nodePlatforms.keys.associateWith { variant ->
    val (os, arch) = variant.split('_', limit = 2)
    val jar = tasks.register<Jar>("pluginVariantJar_$variant") {
        val composed = tasks.composedJar.flatMap { it.archiveFile }
        from(zipTree(composed)) { exclude("META-INF/plugin.xml") }
        from(zipTree(composed)) {
            include("META-INF/plugin.xml")
            filter { line ->
                when {
                    line.trim().startsWith("<version>") -> line.replace("</version>", "-$os-$arch</version>")
                    line.trim() == "<depends>com.intellij.modules.platform</depends>" ->
                        "$line\n  <depends>com.intellij.modules.os.$os</depends>\n  <depends>com.intellij.modules.arch.$arch</depends>"
                    else -> line
                }
            }
        }
        archiveClassifier = "$os-$arch"
        destinationDirectory = layout.buildDirectory.dir("variant-jars")
    }
    tasks.register<Zip>("buildPluginVariant_$variant") {
        group = "build"
        description = "Builds the plugin distribution for $os $arch with its Node.js runtime"
        val base = tasks.buildPlugin.flatMap { it.archiveFile }
        from(zipTree(base)) { exclude("*/lib/${pluginName.get()}-*.jar") }
        from(jar) { into(pluginName.map { "$it/lib" }) }
        // Zip does not keep source modes; the IDE's installer restores what the entry says, so the runtime must be marked here
        from(fetchNode[variant]!!) {
            into(pluginName)
            filesMatching("**/node/node") { permissions { unix("rwxr-xr-x") } }
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

// SHA-256 of every distribution zip, next to them, for the release notes
val checksums by tasks.registering {
    group = "build"
    description = "Writes SHA256SUMS for the plugin distributions"
    val dir = layout.buildDirectory.dir("distributions")
    dependsOn(tasks.buildPlugin, buildPluginVariants)
    inputs.files(dir.map { it.asFileTree.matching { include("*.zip") } })
    outputs.file(dir.map { it.file("SHA256SUMS") })
    doLast {
        val d = dir.get().asFile
        val zips = d.listFiles { f -> f.extension == "zip" }!!.sortedBy { it.name }
        d.resolve("SHA256SUMS").writeText(zips.joinToString("") { f ->
            MessageDigest.getInstance("SHA-256").digest(f.readBytes()).joinToString("") { "%02x".format(it) } + "  ${f.name}\n"
        })
    }
}
