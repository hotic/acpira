package com.github.hotic.acpira.ui

import com.github.hotic.acpira.Acpira
import com.intellij.openapi.Disposable
import com.github.hotic.acpira.web.BrowserProvider
import java.awt.BorderLayout
import javax.swing.JComponent
import javax.swing.JPanel

// Missing plugin classes indicate an installation failure, distinct from an unsupported runtime.
object JcefProbe {
    fun mount(root: JPanel, parent: Disposable, build: (BrowserProvider) -> JComponent) {
        val component = try {
            val provider = BrowserProvider.EP.extensionList.firstOrNull { it.isApplicable() }
            if (provider != null && provider.isSupported()) build(provider)
            else StatusPanel.message("Acpira needs the embedded browser (JCEF), which this IDE runtime does not provide.",
                StatusPanel.JCEF_HINT) { mount(root, parent, build) }
        } catch (error: LinkageError) {
            Acpira.LOG.warn("JCEF plugin classes could not be loaded", error)
            StatusPanel.message("Acpira could not load the embedded browser plugin.",
                "Update Acpira and enable Web Browser (JCEF), then restart the IDE.")
        } catch (error: Exception) {
            Acpira.LOG.warn("browser init failed", error)
            StatusPanel.message("Acpira could not start the embedded browser.",
                "See the IDE log for the initialization error.") { mount(root, parent, build) }
        }
        swap(root, component)
    }

    fun swap(root: JPanel, component: JComponent) {
        if (root.componentCount == 1 && root.getComponent(0) === component) return
        root.removeAll()
        root.add(component, BorderLayout.CENTER)
        root.revalidate()
        root.repaint()
    }
}
