package cc.hotic.acpira.ui.files

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.animateScrollBy
import androidx.compose.foundation.gestures.scrollBy
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.IconButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.platform.UriHandler
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.text.style.TextOverflow
import cc.hotic.acpira.design.Lucide
import cc.hotic.acpira.design.Metrics
import cc.hotic.acpira.design.Palette
import cc.hotic.acpira.design.Type
import cc.hotic.acpira.design.WebMetrics
import cc.hotic.acpira.design.WebType
import cc.hotic.acpira.design.readableWidth
import cc.hotic.acpira.i18n.t
import cc.hotic.acpira.logic.FileLink
import cc.hotic.acpira.logic.FileLinks
import cc.hotic.acpira.logic.code
import cc.hotic.acpira.model.DiffLine
import cc.hotic.acpira.model.ProjectChange
import cc.hotic.acpira.model.ProjectError
import cc.hotic.acpira.store.AppModel
import cc.hotic.acpira.store.FileContent
import cc.hotic.acpira.store.Screen
import cc.hotic.acpira.store.changeStarts
import cc.hotic.acpira.store.fileName
import cc.hotic.acpira.store.folderOf
import cc.hotic.acpira.store.isMarkdownPath
import cc.hotic.acpira.store.joinPath
import cc.hotic.acpira.store.markdownPreviewText
import cc.hotic.acpira.ui.DiffStatText
import cc.hotic.acpira.ui.FileDownload
import cc.hotic.acpira.ui.MediaLayer
import cc.hotic.acpira.ui.Prose
import cc.hotic.acpira.ui.ToolPageBar
import cc.hotic.acpira.ui.opensDirectly
import cc.hotic.acpira.ui.prose.codeRuns
import cc.hotic.acpira.ui.prose.rememberDiffTokens
import kotlinx.coroutines.launch

// Ported from apps/ios Files/FileScreens.swift: the review on the phone (panel/FileTree.tsx's changes,
// panel/FileViewer.tsx), read-only. The files the session changed with their counts, and one file as it is now with
// the removed lines put back and the added ones tinted, a change navigator in the bar. A file link in a reply opens the
// same page at its line. What the page cannot show as text downloads through the media layer's FileDownload.

/** The files the session's edits left changed */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ChangesScreen(model: AppModel, session: String) {
  var files by remember(session) { mutableStateOf<List<ProjectChange>?>(null) }
  var refreshing by remember { mutableStateOf(false) }
  val scope = rememberCoroutineScope()
  LaunchedEffect(session) { files = model.files.changes(session) ?: emptyList() }
  Scaffold(containerColor = Palette.formPage, topBar = { ToolPageBar(t("panel.changes"), "changes", model::pop, ground = Palette.formPage) }) { padding ->
    PullToRefreshBox(
      isRefreshing = refreshing,
      onRefresh = { scope.launch { refreshing = true; model.files.changes(session)?.let { files = it }; refreshing = false } },
      modifier = Modifier.fillMaxSize().padding(padding),
    ) {
      val list = files
      when {
        list == null -> Spinner()
        list.isEmpty() -> Text(
          t("mobile.changes.none"), color = Palette.fg3, fontSize = Type.text2,
          modifier = Modifier.fillMaxWidth().verticalScroll(rememberScrollState()).padding(Metrics.pad).testTag("changes.none"),
        )
        else -> LazyColumn(Modifier.fillMaxSize().readableWidth().testTag("changes.list"), contentPadding = PaddingValues(Metrics.pad)) {
          item {
            // The header: how many files, and the counts summed over them
            Row(
              Modifier.padding(horizontal = Metrics.pad).padding(bottom = Metrics.s2),
              horizontalArrangement = Arrangement.spacedBy(Metrics.s1_5), verticalAlignment = Alignment.CenterVertically,
            ) {
              Text(t("panel.fileCount", "n" to list.size), color = Palette.fg3, fontSize = Type.text3)
              androidx.compose.material3.ProvideTextStyle(WebType.text3) { DiffStatText(list.sumOf { it.add }, list.sumOf { it.del }) }
            }
          }
          itemsIndexed(list, key = { _, c -> c.path }) { i, change ->
            val first = i == 0
            val last = i == list.size - 1
            ChangeRow(change, first, last) { model.push(Screen.File(session, change.path)) }
          }
        }
      }
    }
  }
}

/** One changed file: its name, the folder under it, the counts at the end; a deleted file is struck through */
@Composable
private fun ChangeRow(change: ProjectChange, first: Boolean, last: Boolean, onClick: () -> Unit) {
  val shape = androidx.compose.foundation.shape.RoundedCornerShape(
    topStart = if (first) Metrics.rCardRound else Metrics.s0, topEnd = if (first) Metrics.rCardRound else Metrics.s0,
    bottomStart = if (last) Metrics.rCardRound else Metrics.s0, bottomEnd = if (last) Metrics.rCardRound else Metrics.s0,
  )
  Column(Modifier.fillMaxWidth().background(Palette.formRow, shape)) {
    Row(
      Modifier.fillMaxWidth().clickable(onClick = onClick).heightIn(min = Metrics.formRowHeight)
        .padding(horizontal = Metrics.pad, vertical = Metrics.s2).testTag("changes.row:${change.path}"),
      verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(Metrics.gap),
    ) {
      Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(Metrics.s0_5)) {
        Text(
          fileName(change.path), color = Palette.fg1, fontSize = Type.text1, maxLines = 1, overflow = TextOverflow.MiddleEllipsis,
          textDecoration = if (change.gone == true) TextDecoration.LineThrough else null,
        )
        val dir = folderOf(change.path)
        if (dir.isNotEmpty()) Text(dir, color = Palette.fg3, fontSize = Type.text3, maxLines = 1, overflow = TextOverflow.StartEllipsis)
      }
      androidx.compose.material3.ProvideTextStyle(WebType.text2) { DiffStatText(change.add, change.del) }
      Lucide("chevronRight", Palette.fg3, size = Metrics.chevron)
    }
    if (!last) androidx.compose.material3.HorizontalDivider(color = Palette.line, modifier = Modifier.padding(start = Metrics.pad))
  }
}

/**
 * The last preview choice, carried to the next Markdown file opened (FileViewer.tsx previewMarkdown). Rendered first:
 * on a phone the source is the slower way to read a document
 */
private var previewMarkdown = true

/**
 * One project file, read-only, its changes marked; opens at [line] or else at the first change. A Markdown file opens
 * rendered unless a link named its line, with a bar toggle back to the marked source. A video or a picture, and a file
 * that is binary or over the text limit, offers its download instead (FileDownload)
 */
@Composable
fun FileScreen(model: AppModel, session: String, path: String, line: Int?) {
  val direct = remember(path) { opensDirectly(path) }
  var content by remember(session, path) { mutableStateOf<FileContent?>(null) }
  // A linked line can only be placed in the source
  var preview by remember(path) { mutableStateOf(isMarkdownPath(path) && line == null && previewMarkdown) }
  LaunchedEffect(session, path) { if (!direct) content = model.files.file(session, path) }
  val lines = (content as? FileContent.Lines)?.lines
  val starts = remember(lines) { lines?.let(::changeStarts).orEmpty() }
  val listState = rememberLazyListState()
  val scope = rememberCoroutineScope()
  /** The change the navigator last went to */
  var at by remember(path) { mutableStateOf<Int?>(null) }
  fun go(step: Int) {
    if (starts.isEmpty()) return
    val current = at?.let { starts.indexOf(it) }?.takeIf { it >= 0 } ?: if (step > 0) -1 else 0
    val next = starts[(current + step + starts.size) % starts.size]
    at = next
    scope.launch { listState.centre(next, animate = true) }
  }
  Scaffold(
    containerColor = Palette.bg0,
    topBar = {
      ToolPageBar(fileName(path), "file", model::pop) {
        if (lines != null && !preview && starts.isNotEmpty()) {
          IconButton(onClick = { go(-1) }, modifier = Modifier.testTag("file.prev")) {
            Lucide("chevronUp", Palette.fg1, size = Metrics.iconCtl, contentDescription = t("panel.prevChange"))
          }
          IconButton(onClick = { go(1) }, modifier = Modifier.testTag("file.next")) {
            Lucide("chevronDown", Palette.fg1, size = Metrics.iconCtl, contentDescription = t("panel.nextChange"))
          }
        }
        if (lines != null && isMarkdownPath(path)) {
          IconButton(onClick = { preview = !preview; previewMarkdown = preview }, modifier = Modifier.testTag("file.preview")) {
            // The glyph names where the toggle goes: the source from the preview, the preview from the source
            Lucide(if (preview) "code" else "bookOpen", Palette.fg1, size = Metrics.iconCtl, contentDescription = t("panel.markdownPreview"))
          }
        }
      }
    },
  ) { padding ->
    Box(Modifier.fillMaxSize().padding(padding)) {
      val c = content
      when {
        direct -> Download(model, session, path, null)
        c is FileContent.Failed && (c.error == ProjectError.Binary || c.error == ProjectError.TooLarge) ->
          Download(model, session, path, message(c.error))
        c is FileContent.Lines && preview -> MarkdownPreview(model, session, path, c.lines)
        c is FileContent.Lines -> SourceRows(c.lines, path, line, starts, listState)
        c is FileContent.Failed -> Text(
          message(c.error), color = Palette.fg3, fontSize = Type.text2, textAlign = TextAlign.Center,
          modifier = Modifier.align(Alignment.Center).padding(Metrics.pad).testTag("file.error"),
        )
        else -> Spinner()
      }
    }
  }
}

@Composable
private fun Download(model: AppModel, session: String, path: String, reason: String?) {
  MediaLayer(session, model) {
    Box(Modifier.fillMaxSize().navigationBarsPadding().readableWidth()) { FileDownload(model, session, path, reason) }
  }
}

/** The file's rows: line numbers and signs in a gutter, tinted add / del rows, soft-wrapped code coloured by its grammar */
@Composable
private fun SourceRows(lines: List<DiffLine>, path: String, line: Int?, starts: List<Int>, state: LazyListState) {
  val tokens = rememberDiffTokens(lines, path, inSheet = true)
  val density = LocalDensity.current
  val ch = with(density) { (WebType.mono.fontSize.toPx() * WebType.ch).toDp() }
  val digits = remember(lines) { (lines.mapNotNull { it.newLine ?: it.oldLine }.maxOrNull() ?: 1).toString().length }
  LaunchedEffect(lines) {
    // A linked line, else the first change, comes into view
    val target = line?.let { n -> lines.indexOfFirst { it.newLine == n }.takeIf { it >= 0 } } ?: starts.firstOrNull()
    if (target != null) state.centre(target, animate = false)
  }
  SelectionContainer {
    LazyColumn(
      Modifier.fillMaxSize().testTag("file.rows"), state = state,
      contentPadding = PaddingValues(top = Metrics.gapHalf, bottom = Metrics.gapHalf),
    ) {
      itemsIndexed(lines) { i, row ->
        val tint = when (row.kind) {
          DiffLine.Kind.add -> Palette.diffAdd
          DiffLine.Kind.del -> Palette.diffDel
          else -> Color.Transparent
        }
        Row(Modifier.fillMaxWidth().heightIn(min = WebMetrics.codeOutputLine).background(tint)) {
          Text(
            (if (row.kind == DiffLine.Kind.del) null else row.newLine)?.toString().orEmpty(),
            color = Palette.fg3, style = WebType.codeOutput, textAlign = TextAlign.End, maxLines = 1,
            modifier = Modifier.width(ch * (digits + 1)),
          )
          Text(
            when (row.kind) { DiffLine.Kind.add -> "+"; DiffLine.Kind.del -> "−"; else -> " " },
            color = when (row.kind) { DiffLine.Kind.add -> Palette.ok; DiffLine.Kind.del -> Palette.danger; else -> Palette.fg3 },
            style = WebType.codeOutput, textAlign = TextAlign.Center, maxLines = 1, modifier = Modifier.width(ch * 2),
          )
          // Soft wrap: a phone column cannot scroll sideways inside a page that scrolls down
          val runs = tokens?.getOrNull(i)
          Text(
            if (runs != null && row.code.isNotEmpty()) codeRuns(runs) else AnnotatedString(row.code.ifEmpty { "​" }),
            color = Palette.fg1, style = WebType.codeOutput, modifier = Modifier.weight(1f).padding(end = Metrics.pad),
          )
        }
      }
    }
  }
}

/** The file as it is now, rendered the way a reply is; its file links open from the file's folder */
@Composable
private fun MarkdownPreview(model: AppModel, session: String, path: String, lines: List<DiffLine>) {
  val text = remember(lines) { markdownPreviewText(lines.filter { it.kind != DiffLine.Kind.del }.joinToString("\n") { it.code }) }
  val parent = LocalUriHandler.current
  val scope = rememberCoroutineScope()
  val handler = remember(parent, path) {
    object : UriHandler {
      override fun openUri(uri: String) {
        val link = FileLinks.decode(uri) ?: return parent.openUri(uri)
        scope.launch { openFrom(model, session, path, link) }
      }
    }
  }
  CompositionLocalProvider(LocalUriHandler provides handler) {
    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).navigationBarsPadding().readableWidth().padding(Metrics.pad).testTag("file.preview.body")) {
      Prose(text)
    }
  }
}

/**
 * FileViewer's link from a preview (ToolPanel.tsx openMarkdownLink): a relative path from the file's folder, or from
 * the project's root when nothing is there
 */
private suspend fun openFrom(model: AppModel, session: String, path: String, link: FileLink) {
  var target = link.path
  val near = joinPath(folderOf(path), link.path)
  if (near != null && near != link.path) {
    val read = model.files.file(session, near)
    if (!(read is FileContent.Failed && read.error == ProjectError.Missing)) target = near
  }
  model.push(Screen.File(session, target, link.line))
}

/** Brings row [index] to the middle of the list */
private suspend fun LazyListState.centre(index: Int, animate: Boolean) {
  if (animate) animateScrollToItem(index) else scrollToItem(index)
  val info = layoutInfo
  val item = info.visibleItemsInfo.firstOrNull { it.index == index } ?: return
  val by = -((info.viewportEndOffset - info.viewportStartOffset) / 2f - item.size / 2f)
  if (animate) animateScrollBy(by) else scrollBy(by)
}

@Composable
private fun Spinner() {
  Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
    CircularProgressIndicator(Modifier.size(Metrics.icon), color = Palette.fg2, strokeWidth = Metrics.s0_5)
  }
}

private fun message(error: ProjectError): String = when (error) {
  ProjectError.Missing -> t("panel.error.missing")
  ProjectError.Binary -> t("panel.error.binary")
  ProjectError.TooLarge -> t("panel.error.tooLarge")
  ProjectError.Outside -> t("panel.error.outside")
  ProjectError.NoProject -> t("panel.error.other", "error" to "no project")
  is ProjectError.Message -> t("panel.error.other", "error" to error.text)
}
