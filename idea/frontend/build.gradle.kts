// Frontend module owns JCEF, IDE UI, and the bundled webview while calling backend services only through shared RPC.
dependencies {
    intellijPlatform {
        intellijIdea(providers.gradleProperty("platformVersion"))
        bundledModule("intellij.platform.frontend")
        testFramework(org.jetbrains.intellij.platform.gradle.TestFrameworkType.Platform)
    }

    compileOnly(project(":shared"))
    compileOnly("org.jetbrains.kotlinx:kotlinx-serialization-core-jvm:1.9.0")
    compileOnly("org.jetbrains.kotlinx:kotlinx-serialization-json-jvm:1.9.0")
    testImplementation("junit:junit:4.13.2")
}

val webviewDist: File = file("../../dist/webview")

fun requireBuilt(file: File, what: String): Action<Task> {
    val f = file
    return Action { check(f.isFile) { "$what is missing ($f): run `pnpm build` in the repository root first" } }
}

tasks.processResources {
    doFirst(requireBuilt(webviewDist.resolve("main.js"), "the webview bundle"))
    from(webviewDist) { into("webview") }
}
