package com.github.hotic.acpira.web

import com.github.hotic.acpira.rpc.SidecarState
import com.google.gson.JsonElement
import com.intellij.openapi.Disposable
import com.intellij.openapi.extensions.ExtensionPointName
import com.intellij.openapi.project.Project
import javax.swing.JComponent

// The shell does not load JCEF classes. Each platform layout supplies a browser through its own module classloader.
interface BrowserView {
    val component: JComponent
}

interface BrowserProvider {
    fun isApplicable(): Boolean
    fun isSupported(): Boolean
    fun create(project: Project, host: String, initial: JsonElement?, parent: Disposable,
               onState: (SidecarState, String?) -> Unit,
               onSession: ((String, String) -> Unit)? = null): BrowserView

    companion object {
        val EP = ExtensionPointName.create<BrowserProvider>("com.github.hotic.acpira.browserProvider")
    }
}
