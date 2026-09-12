// Backend module owns the real project, sidecar process, filesystem work, and native RPC provider.
dependencies {
    intellijPlatform {
        intellijIdea(providers.gradleProperty("platformVersion"))
        bundledModule("intellij.platform.backend")
        bundledModule("intellij.platform.kernel.backend")
        bundledModule("intellij.platform.rpc.backend")
        testFramework(org.jetbrains.intellij.platform.gradle.TestFrameworkType.Platform)
    }

    compileOnly(project(":shared"))
    compileOnly("org.jetbrains.kotlinx:kotlinx-serialization-core-jvm:1.9.0")
    compileOnly("org.jetbrains.kotlinx:kotlinx-serialization-json-jvm:1.9.0")
    testImplementation("junit:junit:4.13.2")
}
