package com.github.hotic.acpira.web

import com.intellij.openapi.editor.colors.EditorColorsManager
import com.intellij.ui.ColorUtil
import com.intellij.ui.JBColor
import com.intellij.util.ui.UIUtil

// The --vscode-* roles the webview's tokens.css reads, taken from the IDE look and feel at page load
class ThemeVars(val dark: Boolean, val foreground: String, val sideBarBackground: String, val editorBackground: String, val border: String, val focus: String, val monoFont: String) {
    companion object {
        fun current(): ThemeVars {
            val scheme = EditorColorsManager.getInstance().globalScheme
            return ThemeVars(
                dark = !JBColor.isBright(),
                foreground = ColorUtil.toHtmlColor(UIUtil.getLabelForeground()),
                sideBarBackground = ColorUtil.toHtmlColor(UIUtil.getPanelBackground()),
                editorBackground = ColorUtil.toHtmlColor(scheme.defaultBackground),
                border = ColorUtil.toHtmlColor(UIUtil.getBoundsColor()),
                focus = ColorUtil.toHtmlColor(UIUtil.getFocusedBorderColor() ?: UIUtil.getBoundsColor()),
                monoFont = scheme.editorFontName,
            )
        }
    }
}

// The page a view loads: what src/host/bridge.ts renders for VS Code, with the JCEF bridge spliced in before the bundle. The CSP allows
// only our origin plus the inline bridge script by nonce; the JSQuery function CEF injects natively is not subject to it
object PageTemplate {
    fun render(page: ViewPage): String {
        val t = page.theme
        val csp = listOf(
            "default-src 'none'",
            "img-src 'self' https: data:",
            "style-src 'self' 'unsafe-inline'",
            // Vite inlines small font faces as data: URIs
            "font-src 'self' data:",
            "connect-src 'self'",
            "script-src 'self' 'nonce-${page.nonce}' 'wasm-unsafe-eval'",
        ).joinToString("; ")
        return """<!doctype html>
<html lang="${page.locale}">
<head>
<meta charset="utf-8">
<meta http-equiv="Content-Security-Policy" content="$csp">
<meta name="viewport" content="width=device-width, initial-scale=1">
<link rel="stylesheet" href="/webview/main.css">
<style>
html,body,#root{margin:0;padding:0;height:100%;overflow:hidden}
body{--vscode-foreground:${t.foreground};--vscode-sideBar-background:${t.sideBarBackground};--vscode-editor-background:${t.editorBackground};--vscode-widget-border:${t.border};--vscode-focusBorder:${t.focus};--vscode-editor-font-family:${cssFont(t.monoFont)};background:var(--vscode-sideBar-background);color:var(--vscode-foreground)}
.acp-shell :focus,.acp-shell :focus-visible{outline:none!important}
</style>
</head>
<body class="${if (t.dark) "vscode-dark" else "vscode-light"}">
<div id="root"></div>
<script nonce="${page.nonce}">window.__acpira={host:${jsString(page.host)}};
${page.bridgeJs}</script>
<script type="module" src="/webview/main.js"></script>
</body>
</html>"""
    }

    private fun jsString(s: String) = "\"" + s.replace("\\", "\\\\").replace("\"", "\\\"").replace("<", "\\u003c") + "\""
    private fun cssFont(name: String) = "\"" + name.replace("\"", "") + "\", \"SF Mono\", Menlo, Monaco, monospace"
}
