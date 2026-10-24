// Shared contracts are loaded on both sides; serialization stays compile-only because the platform provides its runtime.
dependencies {
    intellijPlatform {
        intellijIdea(providers.gradleProperty("platformVersion"))
    }

    compileOnly("org.jetbrains.kotlinx:kotlinx-serialization-core-jvm:1.9.0")
    compileOnly("org.jetbrains.kotlinx:kotlinx-serialization-json-jvm:1.9.0")
}
