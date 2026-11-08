package cc.hotic.acpira.logic

import android.content.Context
import android.util.Log
import androidx.javascriptengine.JavaScriptIsolate
import androidx.javascriptengine.JavaScriptSandbox
import cc.hotic.acpira.model.DiffLine
import com.google.common.util.concurrent.ListenableFuture
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import java.util.concurrent.Executor
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException

// Ported from apps/ios Logic/Syntax.swift (chat/codeLanguage.ts, codeSyntax.ts and codeDiffHighlight.ts): which grammar
// a file or a fence gets, and its tokens in the webview's colours. The tokens come from shiki itself (the iOS bundle's
// `Resources/shiki`, synced into the assets by `syncSharedAssets`, written by scripts/export/ios-shiki.ts with the
// webview's grammars, themes and regex options), so a line is coloured exactly as the desktop colours it. iOS runs it in
// JavaScriptCore; Android has no JavaScript engine of its own outside a WebView, so it runs in AndroidX's JavaScript
// sandbox (V8 from the WebView provider, in its own process, with no view).

/** A run of a line in one colour; `color` null is the theme's default foreground, drawn in the card's own ink */
data class CodeToken(val text: String, val color: String?)

/** The scheme the tokens are coloured for (light-plus / dark-plus) */
enum class CodeTheme(val id: String) {
  Dark("dark-plus"), Light("light-plus");

  /** The theme's default foreground: tokens in it render in the card's ink, as the webview's plain text nodes do */
  val defaultColor get() = if (this == Dark) "#D4D4D4" else "#000000"

  companion object {
    fun of(dark: Boolean) = if (dark) Dark else Light
  }
}

/** codeLanguage.ts: the grammar a path's extension names; no guess from content or an extensionless name */
fun codeLanguage(path: String): String? {
  val base = path.split('/', '\\').last()
  // /[^.]\.([^.]+)$/: a suffix after the last dot, which neither starts the name nor follows another dot
  val dot = base.lastIndexOf('.')
  if (dot <= 0 || base[dot - 1] == '.' || dot == base.length - 1) return null
  return CodeLanguages.extensions[base.substring(dot + 1).lowercase()]
}

/** codeLanguage.ts fenceLanguage: a fence is coloured only when its info string names a known grammar */
fun fenceLanguage(tag: String?): String? {
  val name = tag?.trim()?.lowercase()?.takeIf { it.isNotEmpty() } ?: return null
  if (name in CodeLanguages.all) return name
  return CodeLanguages.extensions[name] ?: CodeLanguages.fenceAliases[name]
}

/** codeLanguage.ts's tables, exported beside the grammars (`shiki/languages.json`), so the app answers as the webview does */
object CodeLanguages {
  @Volatile private var tables: Triple<Set<String>, Map<String, String>, Map<String, String>>? = null

  /** Reads the tables once; the highlighter attaches the app's assets before any lookup */
  fun load(context: Context) {
    if (tables != null) return
    tables = runCatching {
      val root = Json.parseToJsonElement(context.assets.open("shiki/languages.json").bufferedReader().use { it.readText() }).jsonObject
      fun map(key: String) = root[key]?.jsonObject?.mapValues { it.value.jsonPrimitive.content }.orEmpty()
      Triple(root["languages"]?.jsonArray?.map { it.jsonPrimitive.content }?.toSet().orEmpty(), map("extensions"), map("fenceAliases"))
    }.getOrElse { Triple(emptySet(), emptyMap(), emptyMap()) }
  }

  val all: Set<String> get() = tables?.first.orEmpty()
  val extensions: Map<String, String> get() = tables?.second.orEmpty()
  val fenceAliases: Map<String, String> get() = tables?.third.orEmpty()
}

/**
 * shiki in one sandboxed isolate, created on first use. Calls are serialised; answers are cached by source, so a card
 * that scrolls back in colours at once, and drawing reads the cache without ever waiting on the engine
 */
object SyntaxHighlighter {
  private const val TAG = "AcpiraSyntax"
  private const val CACHE_SIZE = 200
  /** shiki's per-line `tokenizeTimeLimit` in seconds (scripts/export/ios-shiki.ts) */
  private const val TIME_LIMIT_MS = 5000L

  private val lock = Mutex()
  private val cache = object : LinkedHashMap<String, List<List<CodeToken>>>(CACHE_SIZE, 0.75f, false) {
    override fun removeEldestEntry(eldest: MutableMap.MutableEntry<String, List<List<CodeToken>>>?) = size > CACHE_SIZE
  }
  @Volatile private var context: Context? = null
  private var sandbox: JavaScriptSandbox? = null
  private var isolate: JavaScriptIsolate? = null
  private val loaded = HashSet<String>()
  /** The sandbox is missing or broke: plain text from then on, as a failed JavaScriptCore context does on iOS */
  @Volatile private var failed = false

  /** Gives the highlighter the app's assets; cheap, call it from any view that colours code */
  fun attach(context: Context) {
    if (this.context != null) return
    this.context = context.applicationContext
    CodeLanguages.load(context)
  }

  private fun key(code: String, language: String, theme: CodeTheme) = "${theme.id}\u0000$language\u0000$code"

  /** A cached answer only, never tokenizing: what a view can draw at once */
  fun cached(code: String, language: String?, theme: CodeTheme): List<List<CodeToken>>? {
    language ?: return null
    return synchronized(cache) { cache[key(code, language, theme)] }
  }

  /** One token list per source line (codeSyntax.ts tokenizeCode); null without a language or a working highlighter */
  suspend fun tokenize(code: String, language: String?, theme: CodeTheme): List<List<CodeToken>>? {
    language ?: return null
    cached(code, language, theme)?.let { return it }
    if (failed) return null
    return withContext(Dispatchers.Default) {
      lock.withLock {
        cached(code, language, theme)?.let { return@withLock it }
        val js = guarded { ready(language) } ?: return@withLock null
        val source = code.replace("\r\n", "\n")
        val call = "acpiraShiki.tokenize(${jsString(source)}, ${jsString(language)}, ${jsString(theme.id)})"
        // A line that outruns shiki's per-line time limit comes back cut short. Only a call slower than the limit can
        // hold one: most often a grammar's first use, so it runs once more with the regexes compiled, and an answer
        // still that slow is returned but never cached
        var started = System.nanoTime()
        var answer = guarded { js.evaluateJavaScriptAsync(call).await() } ?: return@withLock null
        if ((System.nanoTime() - started) / 1_000_000 > TIME_LIMIT_MS) {
          started = System.nanoTime()
          answer = guarded { js.evaluateJavaScriptAsync(call).await() } ?: return@withLock null
        }
        val truncated = (System.nanoTime() - started) / 1_000_000 > TIME_LIMIT_MS
        val lines = runCatching { parse(answer, theme) }.getOrNull() ?: return@withLock null
        if (!truncated) synchronized(cache) { cache[key(code, language, theme)] = lines }
        lines
      }
    }
  }

  /** Runs an engine step; a failure (not a cancelled view) turns the highlighter off */
  private inline fun <T> guarded(step: () -> T?): T? = try {
    step()
  } catch (e: kotlinx.coroutines.CancellationException) {
    throw e
  } catch (e: Throwable) {
    fail(e)
    null
  }

  private fun fail(error: Throwable) {
    Log.w(TAG, "syntax highlighter unavailable", error)
    failed = true
    runCatching { isolate?.close() }
    runCatching { sandbox?.close() }
    isolate = null
    sandbox = null
  }

  /** The isolate with `language` loaded (lock held) */
  private suspend fun ready(language: String): JavaScriptIsolate? {
    val context = context ?: return null
    var js = isolate
    if (js == null) {
      if (!JavaScriptSandbox.isSupported()) { failed = true; return null }
      val box = JavaScriptSandbox.createConnectedInstanceAsync(context).await()
      sandbox = box
      js = box.createIsolate()
      isolate = js
      val assets = context.assets
      val script = assets.open("shiki/shiki.js").bufferedReader().use { it.readText() }
      val dark = assets.open("shiki/themes/dark-plus.json").bufferedReader().use { it.readText() }
      val light = assets.open("shiki/themes/light-plus.json").bufferedReader().use { it.readText() }
      js.evaluateJavaScriptAsync(script).await()
      js.evaluateJavaScriptAsync("acpiraShiki.init(${jsString("[$dark,$light]")}); 'ok'").await()
      loaded.clear()
    }
    if (language !in loaded) {
      val grammars = runCatching { context.assets.open("shiki/langs/$language.json").bufferedReader().use { it.readText() } }.getOrNull()
        ?: return null
      js.evaluateJavaScriptAsync("acpiraShiki.load(${jsString(language)}, ${jsString(grammars)}); 'ok'").await()
      loaded += language
    }
    return js
  }

  private fun parse(json: String, theme: CodeTheme): List<List<CodeToken>> =
    Json.parseToJsonElement(json).jsonArray.map { line ->
      merge(line.jsonArray.map { pair -> pair.jsonArray.map { it.jsonPrimitive.content } }, theme)
    }

  /**
   * codeSyntax.ts mergeTokens: adjacent tokens of one colour become one run, whitespace joins the run before it, and
   * the theme's default foreground is no colour
   */
  fun merge(line: List<List<String>>, theme: CodeTheme): List<CodeToken> {
    val out = ArrayList<CodeToken>()
    for (pair in line) {
      if (pair.size != 2 || pair[0].isEmpty()) continue
      val text = pair[0]
      val raw = pair[1]
      if (out.isNotEmpty() && text.all(Char::isWhitespace)) {
        out[out.size - 1] = out.last().copy(text = out.last().text + text)
        continue
      }
      val color = if (raw.isNotEmpty() && raw.uppercase() != theme.defaultColor) raw else null
      val last = out.lastOrNull()
      if (last != null && last.color == color) out[out.size - 1] = last.copy(text = last.text + text)
      else out += CodeToken(text, color)
    }
    return out
  }

  /**
   * codeDiffHighlight.ts without a source file: each run of rows between hunk headers is tokenized side by side, the
   * old side for deleted rows and the new side for the rest, so deleted lines never shift the new side's grammar. One
   * entry per row, null for a hunk header or when nothing could be coloured
   */
  suspend fun diff(lines: List<DiffLine>, path: String, theme: CodeTheme): List<List<CodeToken>?>? =
    walkDiff(lines, path) { code, language -> tokenize(code, language, theme) }

  /** The diff's answer from the cache alone, never falling back to tokenization on a miss or an eviction */
  fun cachedDiff(lines: List<DiffLine>, path: String, theme: CodeTheme): List<List<CodeToken>?>? =
    walkDiff(lines, path) { code, language -> cached(code, language, theme) }

  // Inline, so the suspending caller's reads may suspend inside it
  private inline fun walkDiff(
    lines: List<DiffLine>,
    path: String,
    read: (String, String) -> List<List<CodeToken>>?,
  ): List<List<CodeToken>?>? {
    val language = codeLanguage(path) ?: return null
    val out = arrayOfNulls<List<CodeToken>>(lines.size)
    var start = 0
    for (end in 0..lines.size) {
      if (end != lines.size && lines[end].kind != DiffLine.Kind.hunk) continue
      val segment = (start until end).toList()
      val old = segment.filter { lines[it].kind != DiffLine.Kind.add }
      val new = segment.filter { lines[it].kind != DiffLine.Kind.del }
      if (segment.isNotEmpty()) {
        val oldTokens = read(old.joinToString("\n") { lines[it].code }, language) ?: return null
        val newTokens = read(new.joinToString("\n") { lines[it].code }, language) ?: return null
        old.forEachIndexed { i, row -> if (lines[row].kind == DiffLine.Kind.del && i < oldTokens.size) out[row] = oldTokens[i] }
        new.forEachIndexed { i, row -> if (i < newTokens.size) out[row] = newTokens[i] }
      }
      start = end + 1
    }
    return out.toList()
  }

  /** A JavaScript string literal (JSON's string syntax, with the two line separators JSON leaves bare escaped) */
  private fun jsString(s: String): String =
    JsonPrimitive(s).toString().replace("\u2028", "\\u2028").replace("\u2029", "\\u2029")
}

/**
 * The line's code: the wire text carries its sign column (' ', '-', '+'; transcript/diff.rs), which the gutter draws on
 * its own (codeDiff.ts plainDiffRows); a hunk header stays whole
 */
val DiffLine.code: String get() = if (kind == DiffLine.Kind.hunk) text else text.drop(1)

/** `#rrggbb` (or `#rrggbbaa`) from a theme, as ARGB */
fun themeColor(hex: String): Long? {
  val s = hex.removePrefix("#")
  if (s.length != 6 && s.length != 8) return null
  val v = s.toLongOrNull(16) ?: return null
  val rgb = if (s.length == 8) v shr 8 else v
  val alpha = if (s.length == 8) v and 0xff else 0xff
  return (alpha shl 24) or rgb
}

/** Waits for a future without blocking a thread */
private suspend fun <T> ListenableFuture<T>.await(): T = suspendCancellableCoroutine { cont ->
  addListener({
    try {
      cont.resume(get())
    } catch (e: java.util.concurrent.ExecutionException) {
      cont.resumeWithException(e.cause ?: e)
    } catch (e: Throwable) {
      cont.resumeWithException(e)
    }
  }, DirectExecutor)
  cont.invokeOnCancellation { cancel(false) }
}

private object DirectExecutor : Executor {
  override fun execute(command: Runnable) = command.run()
}
