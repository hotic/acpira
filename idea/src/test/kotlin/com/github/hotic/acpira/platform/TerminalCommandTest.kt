package com.github.hotic.acpira.platform

import org.junit.Assert.assertEquals
import org.junit.Test

class TerminalCommandTest {
    @Test fun `plain words pass through, the rest is single-quoted the way the VS Code host does it`() {
        assertEquals("grok auth login", TerminalCommand.build("grok", listOf("auth", "login"), emptyMap(), windows = false))
        assertEquals(
            "'/Applications/Devin App/devin' auth 'it'\\''s' 'a b'",
            TerminalCommand.build("/Applications/Devin App/devin", listOf("auth", "it's", "a b"), emptyMap(), windows = false),
        )
        assertEquals("bash -c 'curl -fsSL https://x.ai/install.sh | bash'", TerminalCommand.build("bash", listOf("-c", "curl -fsSL https://x.ai/install.sh | bash"), emptyMap(), windows = false))
    }

    @Test fun `a null value unsets the variable around the command, others are set, on POSIX through env`() {
        val env = linkedMapOf<String, String?>("XDG_DATA_HOME" to "/tmp/acpira x", "XDG_CONFIG_HOME" to "/tmp/acpira x", "ACP_BACKEND" to null)
        assertEquals(
            "env -u ACP_BACKEND XDG_DATA_HOME='/tmp/acpira x' XDG_CONFIG_HOME='/tmp/acpira x' devin auth login",
            TerminalCommand.build("devin", listOf("auth", "login"), env, windows = false),
        )
    }

    @Test fun `PowerShell gets env assignments and the call operator`() {
        val env = linkedMapOf<String, String?>("ACP_BACKEND" to null, "XDG_DATA_HOME" to "C:\\Users\\it's")
        assertEquals(
            "\$env:ACP_BACKEND=\$null; \$env:XDG_DATA_HOME='C:\\Users\\it''s'; & 'devin' 'auth' 'login'",
            TerminalCommand.build("devin", listOf("auth", "login"), env, windows = true),
        )
        assertEquals("& 'powershell' '-Command' 'irm https://x | iex'", TerminalCommand.build("powershell", listOf("-Command", "irm https://x | iex"), emptyMap(), windows = true))
    }
}
