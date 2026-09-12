package com.github.hotic.acpira.sidecar

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import java.nio.file.Files
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Assume.assumeTrue
import org.junit.Test
import java.io.File
import java.util.concurrent.TimeUnit

class NodeLocatorTest {
    @Test fun `universal package selects only the backend operating system and architecture`() {
        val dir = Files.createTempDirectory("acpira-node-selection")
        try {
            val cases = listOf(
                Triple("Mac OS X", "aarch64", "mac-arm64/node"),
                Triple("Mac OS X", "x86_64", "mac-x86_64/node"),
                Triple("Linux", "aarch64", "linux-arm64/node"),
                Triple("Linux", "amd64", "linux-x86_64/node"),
                Triple("Windows 11", "arm64", "windows-arm64/node.exe"),
                Triple("Windows 11", "amd64", "windows-x86_64/node.exe"),
            )
            for ((_, _, relative) in cases) {
                val path = dir.resolve(relative)
                Files.createDirectories(path.parent)
                Files.writeString(path, "fixture")
            }
            for ((os, arch, relative) in cases) assertEquals(dir.resolve(relative), NodeLocator.bundledPath(dir, os, arch))
            assertNull(NodeLocator.bundledPath(dir, "Linux", "riscv64"))
            assertNull(NodeLocator.bundledPath(dir, "FreeBSD", "amd64"))
            Files.delete(dir.resolve("linux-x86_64/node"))
            assertNull(NodeLocator.bundledPath(dir, "Linux", "amd64"))
            val legacy = Files.writeString(dir.resolve("node"), "legacy fixture")
            assertEquals(legacy, NodeLocator.bundledPath(dir, "Linux", "amd64"))
        } finally { dir.toFile().deleteRecursively() }
    }

    @Test fun `a process that never closes stdout is killed when the timeout elapses`() {
        assumeTrue(File("/bin/sleep").canExecute())
        val p = ProcessBuilder("/bin/sleep", "30").redirectErrorStream(true).start()
        val t0 = System.currentTimeMillis()
        try {
            NodeLocator.waitForOutput(p, 300, TimeUnit.MILLISECONDS)
            fail("expected timeout")
        } catch (e: SidecarSetupException) {
            assertTrue(e.message!!.contains("did not answer"))
        }
        assertTrue(System.currentTimeMillis() - t0 < 5000)
        p.waitFor(2, TimeUnit.SECONDS)
        assertFalse(p.isAlive)
    }
}
