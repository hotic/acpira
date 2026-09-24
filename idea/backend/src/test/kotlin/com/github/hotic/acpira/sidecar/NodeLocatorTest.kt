package com.github.hotic.acpira.sidecar

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Assume.assumeTrue
import org.junit.Test
import java.io.File
import java.util.concurrent.TimeUnit

class NodeLocatorTest {
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
