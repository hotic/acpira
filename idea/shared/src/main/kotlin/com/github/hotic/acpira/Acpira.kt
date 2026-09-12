package com.github.hotic.acpira

import com.intellij.ide.plugins.cl.PluginAwareClassLoader
import com.intellij.openapi.diagnostic.Logger
import com.intellij.openapi.extensions.PluginDescriptor

object Acpira {
    const val PLUGIN_ID = "com.github.hotic.acpira"
    const val TOOL_WINDOW_ID = "Acpira"
    const val NOTIFICATION_GROUP = "Acpira"

    // The fake https host the webview is served from: a secure context (clipboard, module scripts, fetch) with no init-order race,
    // because the handler is registered on the running CefApp. A real custom scheme is not viable under out-of-process JCEF (see AGENTS.md)
    const val ORIGIN = "https://acpira.local"

    val LOG: Logger = Logger.getInstance("#acpira")

    // Our own descriptor (install path, version) through the plugin class loader: PluginManager's lookups are internal API from 2026.2 on
    val descriptor: PluginDescriptor? get() = (Acpira::class.java.classLoader as? PluginAwareClassLoader)?.pluginDescriptor
    val version: String get() = descriptor?.version ?: "dev"
}
