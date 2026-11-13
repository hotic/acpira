package cc.hotic.acpira.ui.prose

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.compositionLocalOf
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.withStyle
import cc.hotic.acpira.design.LocalDark
import cc.hotic.acpira.design.Metrics
import cc.hotic.acpira.design.Palette
import cc.hotic.acpira.design.WebType
import cc.hotic.acpira.logic.CodeTheme
import cc.hotic.acpira.logic.CodeToken
import cc.hotic.acpira.logic.SyntaxHighlighter
import cc.hotic.acpira.logic.codeLanguage
import cc.hotic.acpira.logic.fenceLanguage
import cc.hotic.acpira.model.DiffLine
import cc.hotic.acpira.logic.themeColor
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

// Ported from apps/ios Prose/Prose.swift CodeSurface and the drawing half of Logic/Syntax.swift (codeRuns, codeSource):
// fenced code on the code background inside the conversation ring, scrolling sideways, coloured when the fence names a
// known grammar (CodeBlock.tsx).

/** CodeBlock.tsx STREAM_SETTLE_MS: a fence that streams asks again this long after its last pass landed */
private const val STREAM_SETTLE_MS = 120L

/** Tokens as one annotated run, each coloured run in its colour and the rest left to the inherited ink */
fun codeRuns(tokens: List<CodeToken>): AnnotatedString = buildAnnotatedString { appendRuns(tokens) }

private fun AnnotatedString.Builder.appendRuns(tokens: List<CodeToken>) {
  for (token in tokens) {
    val argb = token.color?.let(::themeColor)
    if (argb != null) withStyle(SpanStyle(color = Color(argb))) { append(token.text) } else append(token.text)
  }
}

/**
 * A fence's source as the webview shows it (CodeBlock.tsx syntaxChildren): coloured when its tokens are for this text;
 * while it streams on past them, the complete lines they cover keep their colours and the growing tail is plain
 */
fun codeSource(text: String, tokenized: String?, lines: List<List<CodeToken>>?): AnnotatedString {
  if (tokenized == null || lines == null || !text.startsWith(tokenized)) return AnnotatedString(text)
  // The lines to colour: all of them for the same text, else those a newline closed
  val keep = if (text == tokenized) lines.size else minOf(lines.size, tokenized.count { it == '\n' })
  return buildAnnotatedString {
    for (i in 0 until keep) {
      appendRuns(lines[i])
      if (i < keep - 1 || text != tokenized) append('\n')
    }
    if (text != tokenized) append(text.substring(tokenized.lastIndexOf('\n') + 1))
  }
}

/** A pass's tokens and the text they were made for */
private class Colored(val text: String, val language: String, val theme: CodeTheme, val lines: List<List<CodeToken>>, val at: Long)

/** A code block's fill where the stock one clashes with its ground (a screenshot card's sheet); null keeps --bg-code */
val LocalCodeFill = compositionLocalOf<Color?> { null }

/** CodeSurface: one fence, coloured once a pass for its text (or a prefix of it) lands */
@Composable
fun CodeSurface(text: String, lang: String?, modifier: Modifier = Modifier) {
  val context = LocalContext.current
  remember { SyntaxHighlighter.attach(context) }
  val theme = CodeTheme.of(LocalDark.current)
  val language = remember(lang) { fenceLanguage(lang) }
  var colored by remember { mutableStateOf<Colored?>(null) }
  val scope = rememberCoroutineScope()
  val running = remember { booleanArrayOf(false) }
  // A card that scrolls back in, or a theme switch to one seen before, colours from the cache at once
  val usable = colored?.takeIf { it.language == language && it.theme == theme }
    ?: language?.let { l -> SyntaxHighlighter.cached(text, l, theme)?.let { Colored(text, l, theme, it, 0) } }
  val shape = RoundedCornerShape(Metrics.rLg)
  Box(
    modifier.fillMaxWidth()
      .background(LocalCodeFill.current ?: Palette.bgCode, shape)
      .border(Metrics.hairline, Palette.conversationLine, shape)
      // The CSS border takes room inside the card
      .padding(Metrics.hairline)
      .horizontalScroll(rememberScrollState()),
  ) {
    Text(
      if (usable == null) AnnotatedString(text) else codeSource(text, usable.text, usable.lines),
      style = WebType.mono, color = Palette.fg1, softWrap = false,
      modifier = Modifier.padding(horizontal = Metrics.pad, vertical = Metrics.pad - Metrics.s0_5),
    )
  }
  // One pass at a time. A pass for a text the fence has since extended still lands (its lines colour the text it
  // covers), and its landing asks again for the text then: dropped by every chunk, a pass of a streaming fence never
  // landed
  LaunchedEffect(text, language, theme, colored?.at) {
    val l = language ?: return@LaunchedEffect
    if (running[0]) return@LaunchedEffect
    val last = colored
    if (last != null && last.text == text && last.language == l && last.theme == theme) return@LaunchedEffect
    if (last != null && last.language == l && last.theme == theme && text.startsWith(last.text)) {
      delay(maxOf(0L, last.at + STREAM_SETTLE_MS - System.currentTimeMillis()))
    }
    // The pass itself outlives the next chunk: only the card leaving the composition cancels it
    running[0] = true
    scope.launch {
      try {
        val lines = SyntaxHighlighter.tokenize(text, l, theme)
        if (lines != null) colored = Colored(text, l, theme, lines, System.currentTimeMillis())
      } finally {
        running[0] = false
      }
    }
  }
}

/**
 * A diff's syntax colours (ToolViews.swift DiffBlockView): one token list per row, null for a hunk header or a row
 * nothing coloured. The transcript's bounded preview reads the cache at once; the pass runs off the main thread, and a
 * stale pass never replaces a newer source
 */
@Composable
fun rememberDiffTokens(lines: List<DiffLine>, path: String, inSheet: Boolean): List<List<CodeToken>?>? {
  val context = LocalContext.current
  remember { SyntaxHighlighter.attach(context) }
  val theme = CodeTheme.of(LocalDark.current)
  var tokens by remember(lines, path, theme) {
    mutableStateOf(if (inSheet) null else SyntaxHighlighter.cachedDiff(lines, path, theme))
  }
  LaunchedEffect(lines, path, theme) {
    if (tokens != null || codeLanguage(path) == null) return@LaunchedEffect
    SyntaxHighlighter.diff(lines, path, theme)?.let { tokens = it }
  }
  return tokens
}
