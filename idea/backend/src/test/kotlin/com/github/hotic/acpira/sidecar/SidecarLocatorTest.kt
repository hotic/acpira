package com.github.hotic.acpira.sidecar

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.fail
import org.junit.Test
import java.nio.file.Files
import java.nio.file.attribute.PosixFilePermissions

class SidecarLocatorTest {
    private val node = SidecarCommand(listOf("node", "host-server.cjs"), "script host-server.cjs")

    private fun executable(): java.nio.file.Path {
        val f = Files.createTempFile("acpira-bin", "")
        runCatching { Files.setPosixFilePermissions(f, PosixFilePermissions.fromString("rwxr-xr-x")) }
        f.toFile().setExecutable(true)
        return f
    }

    @Test fun `universal package selects only the backend operating system and architecture`() {
        val dir = Files.createTempDirectory("acpira-sidecar-bin")
        try {
            val cases = listOf(
                Triple("Mac OS X", "aarch64", "mac-arm64/acpira"),
                Triple("Mac OS X", "x86_64", "mac-x86_64/acpira"),
                Triple("Linux", "aarch64", "linux-arm64/acpira"),
                Triple("Linux", "amd64", "linux-x86_64/acpira"),
                Triple("Windows 11", "arm64", "windows-arm64/acpira.exe"),
                Triple("Windows 11", "amd64", "windows-x86_64/acpira.exe"),
            )
            for ((_, _, relative) in cases) dir.resolve(relative).also { Files.createDirectories(it.parent); Files.writeString(it, "x") }
            for ((os, arch, relative) in cases) assertEquals(dir.resolve(relative), SidecarLocator.binaryPath(dir, os, arch))
            assertNull(SidecarLocator.binaryPath(dir, "Linux", "riscv64"))
            assertNull(SidecarLocator.binaryPath(dir, "FreeBSD", "amd64"))
            Files.delete(dir.resolve("linux-x86_64/acpira"))
            assertNull(SidecarLocator.binaryPath(dir, "Linux", "amd64"))
        } finally { dir.toFile().deleteRecursively() }
    }

    @Test fun `engine precedence keeps node reachable in every case`() {
        val bin = executable()
        try {
            val env = { map: Map<String, String> -> { k: String -> map[k] } }
            assertEquals(listOf(bin.toString()), SidecarLocator.choose(env(emptyMap()), bin, { node }, {}).argv)
            assertEquals(node, SidecarLocator.choose(env(mapOf("ACPIRA_ENGINE" to "node")), bin, { node }, {}))
            assertEquals(node, SidecarLocator.choose(env(mapOf("ACPIRA_HOST_SERVER" to "/repo/dist/host-server.cjs")), bin, { node }, {}))
            assertEquals(node, SidecarLocator.choose(env(emptyMap()), null, { node }, {}))
            assertEquals(listOf(bin.toString()), SidecarLocator.choose(env(mapOf("ACPIRA_SIDECAR_BIN" to bin.toString(), "ACPIRA_HOST_SERVER" to "/x")), null, { node }, {}).argv)
            try {
                SidecarLocator.choose(env(mapOf("ACPIRA_SIDECAR_BIN" to "/definitely/missing")), null, { node }, {})
                fail("expected a setup error")
            } catch (_: SidecarSetupException) {}
        } finally { Files.deleteIfExists(bin) }
    }
}
