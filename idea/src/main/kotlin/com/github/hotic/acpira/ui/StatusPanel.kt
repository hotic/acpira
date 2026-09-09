package com.github.hotic.acpira.ui

import com.intellij.ui.components.JBLabel
import com.intellij.ui.components.JBPanel
import com.intellij.util.ui.JBUI
import java.awt.BorderLayout
import java.awt.FlowLayout
import javax.swing.JButton
import javax.swing.JComponent
import javax.swing.JPanel
import javax.swing.SwingConstants

// What a view slot shows instead of a browser: no JCEF in this runtime, or a sidecar that cannot start (with a Retry)
object StatusPanel {
    const val JCEF_HINT = "Choose a JetBrains Runtime with JCEF (Help → Find Action → Choose Boot Java Runtime for the IDE) and restart."

    fun message(title: String, detail: String, retry: (() -> Unit)? = null): JComponent {
        val panel = JBPanel<JBPanel<*>>(BorderLayout()).apply { border = JBUI.Borders.empty(16) }
        val text = JBLabel("<html><b>$title</b><br><br>$detail</html>", SwingConstants.LEFT).apply { setAllowAutoWrapping(true); setCopyable(true) }
        panel.add(text, BorderLayout.NORTH)
        if (retry != null) {
            val actions = JPanel(FlowLayout(FlowLayout.LEFT, 0, 0)).apply { border = JBUI.Borders.emptyTop(12) }
            actions.add(JButton("Retry").apply { addActionListener { retry() } })
            panel.add(actions, BorderLayout.CENTER)
        }
        return panel
    }
}
