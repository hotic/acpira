package com.github.hotic.acpira.web

import com.github.hotic.acpira.rpc.SidecarState
import com.google.gson.JsonElement
import com.intellij.openapi.Disposable
import com.intellij.openapi.application.ApplicationInfo
import com.intellij.openapi.project.Project
import com.intellij.ui.jcef.JBCefApp

// 261 exposes JCEF through IDEA CORE; 262+ requires explicit public JCEF content-module dependencies.
// The legacy provider stays loadable on 262 but never touches JCEF there.
abstract class JcefBrowserProvider : BrowserProvider {
    override fun isSupported(): Boolean = JBCefApp.isSupported()
    override fun create(project: Project, host: String, initial: JsonElement?, parent: Disposable,
                        onState: (SidecarState, String?) -> Unit,
                        onSession: ((String, String) -> Unit)?): BrowserView =
        AcpiraBrowser(project, host, initial, parent, onState, onSession)
}

class LegacyJcefBrowserProvider : JcefBrowserProvider() {
    override fun isApplicable() = ApplicationInfo.getInstance().build.baselineVersion < 262
}

class ModularJcefBrowserProvider : JcefBrowserProvider() {
    override fun isApplicable() = ApplicationInfo.getInstance().build.baselineVersion >= 262
}
