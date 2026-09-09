package com.github.hotic.acpira.ui

import com.intellij.openapi.util.IconLoader

// The plugin's own icons (resources/icons, light + `_dark` pairs the loader picks between)
object AcpiraIcons {
    // 13 px glyph: the tool window stripe (plugin.xml) and the session editor tab
    @JvmField val ToolWindow = IconLoader.getIcon("/icons/toolWindow.svg", AcpiraIcons::class.java)
}
