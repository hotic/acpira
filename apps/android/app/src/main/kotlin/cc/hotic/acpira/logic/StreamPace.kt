package cc.hotic.acpira.logic

import java.text.BreakIterator
import kotlin.math.exp

// Ported from apps/ios Logic/StreamPace.swift (chat/streamMotion.ts useSmoothText): the visible prefix of a streamed
// text. The visible text advances at the agent's measured throughput (characters per second, a time-weighted EMA that
// counts stalls too), so a burst after a pause plays at the speed the stream has been running at instead of all at
// once. Lag beyond `lag` raises the target speed proportionally (never above `maxLag` of backlog), the actual speed
// eases toward the target over `ease`, and `finish` flushes the rest once the stream ends. Counts are graphemes, so a
// cut never splits a surrogate pair, a ZWJ emoji or a combining mark. Times are seconds.

/** STREAM_PACE (chosen in lab/stream-pace.preview.html) */
data class StreamPace(
  val tick: Double = 0.032,
  val lag: Double = 0.5,
  val maxLag: Double = 1.5,
  val ease: Double = 0.18,
  val window: Double = 0.8,
  val finish: Double = 0.22,
) {
  companion object {
    val standard = StreamPace()
    /** motion.css --stream-char-duration: one glyph's fade */
    const val GLYPH_FADE = 0.18
  }
}

/** One step of the reveal: at `time` the visible prefix grew from `from` to `to` characters */
data class StreamReveal(val time: Double, val from: Int, val to: Int)

/** A text split into graphemes: `ends[k]` is the UTF-16 end of grapheme k */
class Graphemes private constructor(val text: String, private val ends: IntArray) {
  val size get() = ends.size

  /** The UTF-16 length of the first `n` graphemes */
  fun offset(n: Int): Int = if (n <= 0) 0 else ends[minOf(n, ends.size) - 1]

  fun prefix(n: Int): String = text.substring(0, offset(n))

  /** Grapheme k */
  operator fun get(k: Int): String = text.substring(if (k == 0) 0 else ends[k - 1], ends[k])

  /** Grapheme k when it is one UTF-16 unit, else null */
  fun char(k: Int): Char? {
    val start = if (k == 0) 0 else ends[k - 1]
    return if (ends[k] - start == 1) text[start] else null
  }

  /** The graphemes of `next`, which extends this text: only the tail from the last whole grapheme is split again */
  fun extended(next: String): Graphemes {
    // The last grapheme may grow (a combining mark, a ZWJ sequence), so the split resumes before it
    val keep = maxOf(0, ends.size - 1)
    val from = offset(keep)
    val tail = split(next, from)
    return Graphemes(next, ends.copyOf(keep) + tail)
  }

  companion object {
    val empty = Graphemes("", IntArray(0))

    fun of(text: String) = Graphemes(text, split(text, 0))

    private fun split(text: String, from: Int): IntArray {
      if (from >= text.length) return IntArray(0)
      val it = BreakIterator.getCharacterInstance()
      it.setText(text)
      val out = ArrayList<Int>()
      var end = it.following(from)
      while (end != BreakIterator.DONE) { out += end; end = it.next() }
      return out.toIntArray()
    }
  }
}

/** The pacer's state for one streamed text (a value: every step returns the next state) */
class StreamPacer(text: String = "", streaming: Boolean = false, val pace: StreamPace = StreamPace.standard) {
  /** The received text, as graphemes */
  var target: Graphemes = Graphemes.of(text); private set
  /** Characters on screen */
  var count = if (streaming) 0 else target.size; private set
  var streaming = streaming; private set
  /** Recent reveal steps, newest last, kept for one glyph fade (STREAM_DURATION_MS) */
  var reveals: List<StreamReveal> = emptyList(); private set
  private var pos = count.toDouble()
  private var at = 0.0
  /** Characters per second of the incoming stream, and actually revealed */
  private var throughput = 0.0
  private var speed = 0.0

  /** The visible text still trails the received one */
  val pending get() = count < target.size
  val visible: String get() = target.prefix(count)

  /**
   * A new value of the streamed text. Text that does not extend the previous value (an edit, a replay) or that was
   * already there before the reader looked (`skip`: a session opened mid-turn or the catch-up after returning to it)
   * shows at once and measures the rate from scratch on the next chunk
   */
  fun receive(text: String, streaming: Boolean, now: Double, skip: Boolean = false) {
    val appended = text.startsWith(target.text)
    val next = if (appended) target.extended(text) else Graphemes.of(text)
    val added = next.size - target.size
    target = next
    this.streaming = streaming
    if (!appended || skip) {
      count = next.size; pos = count.toDouble(); speed = 0.0; reveals = emptyList()
      if (skip) at = 0.0
      return
    }
    if (added <= 0) return
    if (at == 0.0) {
      // First chunk: no rate yet, assume it covers one lag budget
      throughput = added / pace.lag
      speed = throughput
    } else {
      // Time-weighted EMA: a stall lowers the measured rate instead of making the next burst look fast
      val gap = (now - at).coerceIn(0.016, pace.maxLag)
      throughput += (added / gap - throughput) * (1 - exp(-gap / pace.window))
    }
    at = now
  }

  /** One pacing tick, `dt` seconds after the last */
  fun tick(now: Double, dt: Double, fade: Double = StreamPace.GLYPH_FADE) {
    expire(now, fade)
    if (!pending) return
    val backlog = target.size - pos
    // Proportional catch-up past the lag budget, a hard floor at maxLag, and a quick flush once the stream ends
    val lag = if (throughput > 0) backlog / throughput else pace.lag
    var want = maxOf(throughput * (1 + maxOf(0.0, lag - pace.lag) / pace.lag), backlog / pace.maxLag)
    if (!streaming) want = maxOf(want, backlog / pace.finish)
    speed += (want - speed) * (1 - exp(-dt / pace.ease))
    pos = minOf(target.size.toDouble(), pos + speed * dt)
    val next = StreamCut.cut(target, pos.toInt(), streaming)
    if (next > count) {
      reveals = reveals + StreamReveal(now, count, next)
      count = next
    }
  }

  /** Drops reveal steps older than one fade without moving the text */
  fun expire(now: Double, fade: Double = StreamPace.GLYPH_FADE) {
    if (reveals.any { now - it.time > fade }) reveals = reveals.filter { now - it.time <= fade }
  }
}

/**
 * Cut points: markdown punctuation and whitespace a visible prefix must not end on. A transient literal ("- *",
 * "**", "1") would otherwise render as text for a tick and then turn into formatting
 */
object StreamCut {
  private const val SYNTAX = "*_`~#>|+=-[]()!\\"
  private const val REACH = 64

  fun endsOnSyntax(text: Graphemes, cut: Int): Boolean {
    if (cut <= 0) return false
    val last = text.char(cut - 1)
    if (last == null) return text[cut - 1].isBlank()
    if (last.isWhitespace() || SYNTAX.indexOf(last) >= 0) return true
    // An ordered-list marker: digits (and a `.` / `)`) after only spaces since a line start. A window that does not
    // reach a line start cannot prove one
    val lo = maxOf(0, cut - 24)
    var i = cut - 1
    if (last == '.' || last == ')') i--
    val digitsEnd = i
    while (i >= lo && text.char(i)?.let { it in '0'..'9' } == true) i--
    if (i >= digitsEnd) return false
    while (i >= lo && (text.char(i) == ' ' || text.char(i) == '\t')) i--
    if (i < lo) return lo == 0
    return text.char(i) == '\n'
  }

  /**
   * Cut point at or after `n` that lands after a content character. While streaming, received text that itself ends on
   * syntax is held back to its last content character until more arrives
   */
  fun cut(text: Graphemes, n: Int, streaming: Boolean): Int {
    var cut = n.coerceIn(0, text.size)
    val limit = minOf(text.size, cut + REACH)
    while (cut in 1 until limit && endsOnSyntax(text, cut)) cut++
    if (streaming && cut == text.size) while (cut > 0 && endsOnSyntax(text, cut)) cut--
    return cut
  }
}

/**
 * The fading glyphs of a streamed text at one frame: a glyph `d` places before the end of the visible text is
 * character `count - d`, and it fades in from the time the reveal step that brought that character in ran
 * (motion.css `sd-acpGlyph`: opacity only, --stream-char-duration, ease). Ported from Prose.swift StreamFade
 */
data class StreamFade(val count: Int, val reveals: List<StreamReveal>) {
  /** Opacity of the glyph `distance` places before the end; 1 once its fade has run */
  fun opacity(distance: Int, now: Double): Float {
    val position = count - distance
    val step = reveals.lastOrNull { it.from < position && position <= it.to } ?: return 1f
    val t = ((now - step.time) / StreamPace.GLYPH_FADE).coerceIn(0.0, 1.0)
    return CSS_EASE.transform(t.toFloat())
  }

  /** How many glyphs from the end can still be fading */
  val reach: Int get() = reveals.firstOrNull()?.let { count - it.from } ?: 0

  private companion object {
    /** CSS `ease`: cubic-bezier(.25, .1, .25, 1) */
    val CSS_EASE = CubicEase(0.25f, 0.1f, 0.25f, 1f)
  }
}

/** A CSS cubic-bezier timing function, free of Compose so the logic stays testable on the JVM */
class CubicEase(private val x1: Float, private val y1: Float, private val x2: Float, private val y2: Float) {
  private fun bezier(t: Float, a: Float, b: Float): Float {
    val u = 1 - t
    return 3 * u * u * t * a + 3 * u * t * t * b + t * t * t
  }

  fun transform(x: Float): Float {
    if (x <= 0f) return 0f
    if (x >= 1f) return 1f
    // Bisection on x(t): monotonic for control points inside [0, 1]
    var lo = 0f
    var hi = 1f
    var t = x
    repeat(24) {
      val v = bezier(t, x1, x2)
      if (v < x) lo = t else hi = t
      t = (lo + hi) / 2
    }
    return bezier(t, y1, y2)
  }
}
