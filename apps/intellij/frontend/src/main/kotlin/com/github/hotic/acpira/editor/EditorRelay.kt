package com.github.hotic.acpira.editor

import com.github.hotic.acpira.connection.AcpiraConnection
import com.github.hotic.acpira.ui.AcpiraSessionFile
import com.google.gson.JsonElement
import com.google.gson.JsonObject
import com.intellij.openapi.Disposable
import com.intellij.openapi.application.ApplicationManager
import com.intellij.openapi.components.Service
import com.intellij.openapi.components.service
import com.intellij.openapi.editor.Editor
import com.intellij.openapi.editor.EditorFactory
import com.intellij.openapi.editor.event.EditorFactoryEvent
import com.intellij.openapi.editor.event.EditorFactoryListener
import com.intellij.openapi.editor.event.SelectionListener
import com.intellij.openapi.fileEditor.FileDocumentManager
import com.intellij.openapi.fileEditor.FileEditorManager
import com.intellij.openapi.fileEditor.FileEditorManagerEvent
import com.intellij.openapi.fileEditor.FileEditorManagerListener
import com.intellij.openapi.fileEditor.TextEditor
import com.intellij.openapi.ide.CopyPasteManager
import com.intellij.openapi.project.Project
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import java.awt.datatransfer.DataFlavor
import java.nio.file.Path

// The IDE editor's selection and copies, relayed to this project's chat views (the VS Code shell does the same in extension.ts).
// Focus moving into the chat (the tool window or an Acpira editor tab) keeps the last selection, since that is when it gets used;
// another file editor becoming active, a collapsed selection or closing the editor clears it. Selection events are debounced:
// a drag fires dozens. The relay starts with whatever the selected editor already holds, and keeps the last copy for pages
// that open later.
// Only local files are offered: a split-mode client's remote VFS path is not a path the backend's agent can read (unverified there)
@Service(Service.Level.PROJECT)
class EditorRelay(private val project: Project, private val cs: CoroutineScope) : Disposable {
    @Volatile private var current: JsonObject? = null
    @Volatile private var lastCopy: JsonObject? = null
    private var pending: Job? = null

    init {
        val factory = EditorFactory.getInstance()
        factory.eventMulticaster.addSelectionListener(object : SelectionListener {
            override fun selectionChanged(e: com.intellij.openapi.editor.event.SelectionEvent) {
                if (e.editor.project == project) schedule(selectionOf(e.editor))
            }
        }, this)
        factory.addEditorFactoryListener(object : EditorFactoryListener {
            override fun editorReleased(event: EditorFactoryEvent) {
                val uri = uriOf(event.editor) ?: return
                if (current?.get("uri")?.asString == uri) schedule(null)
            }
        }, this)
        project.messageBus.connect(this).subscribe(FileEditorManagerListener.FILE_EDITOR_MANAGER, object : FileEditorManagerListener {
            override fun selectionChanged(event: FileEditorManagerEvent) {
                val editor = (event.newEditor as? TextEditor)?.editor
                when {
                    editor != null -> schedule(selectionOf(editor))
                    // An Acpira chat tab is where the selection gets used; any other editor (image preview, diff…) drops it
                    event.newFile !is AcpiraSessionFile -> schedule(null)
                }
            }
        })
        // A copy in an editor: the clipboard now holds exactly the selected text of this project's active editor
        CopyPasteManager.getInstance().addContentChangedListener({ _, now ->
            val copied = runCatching { now?.getTransferData(DataFlavor.stringFlavor) as? String }.getOrNull() ?: return@addContentChangedListener
            val editor = FileEditorManager.getInstance(project).selectedTextEditor ?: return@addContentChangedListener
            val range = selectionOf(editor) ?: return@addContentChangedListener
            if (normalize(range.get("text").asString) != normalize(copied)) return@addContentChangedListener
            lastCopy = range
            connection().shell(message("editorCopy", range))
        }, this)
        // A selection made before this relay existed (the chat opened after it) is offered too
        ApplicationManager.getApplication().invokeLater({
            FileEditorManager.getInstance(project).selectedTextEditor?.let { schedule(selectionOf(it)) }
        }, project.disposed)
    }

    fun current(): JsonObject? = current

    fun lastCopy(): JsonObject? = lastCopy

    // The selection of an editor, or the caret's line when nothing is selected and `lineFallback` asks for it ("Add to chat")
    fun selectionOf(editor: Editor, lineFallback: Boolean = false): JsonObject? {
        val uri = uriOf(editor) ?: return null
        val doc = editor.document
        val sel = editor.selectionModel
        val (start, end) = when {
            sel.hasSelection() -> sel.selectionStart to sel.selectionEnd
            lineFallback -> editor.caretModel.logicalPosition.line.let { doc.getLineStartOffset(it) to doc.getLineEndOffset(it) }
            else -> return null
        }
        if (end - start > EditorRange.MAX_TEXT_BYTES) return null
        val startLine = doc.getLineNumber(start)
        val endLine = doc.getLineNumber(end)
        return EditorRange.of(uri, startLine, endLine, end == doc.getLineStartOffset(endLine), doc.charsSequence.subSequence(start, end).toString())
    }

    private fun schedule(next: JsonObject?) {
        pending?.cancel()
        pending = cs.launch {
            delay(150)
            if (current == null && next == null) return@launch
            current = next
            connection().shell(message("editorSelection", next))
        }
    }

    private fun connection() = project.service<AcpiraConnection>()

    private fun uriOf(editor: Editor): String? {
        val file = FileDocumentManager.getInstance().getFile(editor.document) ?: return null
        if (!file.isInLocalFileSystem) return null
        return runCatching { Path.of(file.path).toUri().toASCIIString() }.getOrNull()
    }

    override fun dispose() {
        pending?.cancel()
    }

    companion object {
        private fun normalize(text: String) = text.replace("\r\n", "\n").replace('\r', '\n').trimEnd('\n')

        // `{ type, selection }`; a null selection is left out (the live selection cleared)
        fun message(type: String, selection: JsonObject?): JsonElement = JsonObject().apply {
            addProperty("type", type)
            if (selection != null) add("selection", selection)
        }
    }
}
