package cc.hotic.acpira.design

import android.annotation.SuppressLint
import android.content.Context
import android.content.SharedPreferences
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.PixelFormat
import android.graphics.RectF
import android.graphics.Typeface
import android.os.Build
import android.os.SystemClock
import android.os.Trace
import android.util.Log
import android.util.TypedValue
import android.view.Choreographer
import android.view.Gravity
import android.view.MotionEvent
import android.view.View
import android.view.ViewConfiguration
import android.view.ViewTreeObserver
import android.view.WindowManager
import androidx.activity.ComponentActivity
import androidx.annotation.RequiresApi
import androidx.core.graphics.ColorUtils
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat
import androidx.lifecycle.DefaultLifecycleObserver
import androidx.lifecycle.LifecycleOwner
import cc.hotic.acpira.i18n.t
import kotlin.math.abs
import kotlin.math.ceil
import kotlin.math.roundToInt

/**
 * Settings → Developer: a frame-rate and hitch overlay for judging smoothness on the phone itself, release builds
 * included (apps/ios Design/PerfHUD.swift). It is a small window of its own, added through the activity's window
 * manager: every app window (a dialog, a bottom sheet) shares one layer and the newest sits on top, so each time the
 * activity's window loses focus to a new one the overlay is added again above it. A drag moves it, a tap clears its
 * counters.
 *
 * Timing comes from one Choreographer callback per frame. The expected interval is the frame's own (apps/ios
 * `targetTimestamp - timestamp`): on Android 13 and later the spacing of the frame's presentation timelines, which
 * follows the app's actual frame rate (a lowered rate, a 120 Hz panel running the app at 60) rather than the panel's
 * nominal one; before that the display's refresh rate. The switches can also be set for one launch with the extras
 * `hud.fps` / `hud.hitches` (`YES` / `NO`, like the iOS app's `-acpira.hud.fps YES`). A frame counts as a hitch
 * when it came more than half a frame late (interval over 1.5x expected), and its hitch time is how late it came. Each hitch is logged (tag `AcpiraHitch`) and set on a trace counter, so Perfetto lines it up with the
 * rest of a trace. The overlay is a plain View: it never touches the app's composition, and redraws a few times a
 * second plus on each hitch.
 */
class PerfHud private constructor(private val activity: ComponentActivity) :
  DefaultLifecycleObserver, Choreographer.FrameCallback {

  companion object {
    /** The switches' SharedPreferences file and keys (apps/ios `Prefs.hudFPS` / `Prefs.hudHitches`) */
    const val PREFS = "developer"
    const val FPS = "hud.fps"
    const val HITCHES = "hud.hitches"
    private const val TAG = "AcpiraHitch"

    /** The launch extras' values, over the stored switches for this process until a switch is flipped */
    private val launched = mutableMapOf<String, Boolean>()
    /** The overlays alive, told when a switch flips without changing its stored value */
    private val huds = mutableSetOf<PerfHud>()

    /** Follows the two switches and the activity's lifecycle. Call once from the activity's onCreate */
    fun install(activity: ComponentActivity, fps: Boolean? = null, hitches: Boolean? = null) {
      fps?.let { launched[FPS] = it }
      hitches?.let { launched[HITCHES] = it }
      activity.lifecycle.addObserver(PerfHud(activity))
    }

    /** A switch as it stands: the launch extra's value, else the stored one */
    fun isOn(prefs: SharedPreferences, key: String): Boolean = launched[key] ?: prefs.getBoolean(key, false)

    /** Flips a switch: stored, and the launch extra's value for it dropped */
    fun set(prefs: SharedPreferences, key: String, on: Boolean) {
      launched.remove(key)
      prefs.edit().putBoolean(key, on).apply()
      huds.forEach { it.sync() }
    }
  }

  private val prefs = activity.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
  // Held here: SharedPreferences keeps its listeners only weakly
  private val prefsListener = SharedPreferences.OnSharedPreferenceChangeListener { _, _ -> sync() }
  // A dialog or sheet just took focus as a window above the overlay: put the overlay back on top
  private val focusListener = ViewTreeObserver.OnWindowFocusChangeListener { focused -> if (!focused) decor.post { raise() } }
  private val wm get() = activity.windowManager
  private val decor get() = activity.window.decorView

  private var started = false
  private var showFps = false
  private var showHitches = false
  private var view: HudView? = null
  private var added = false
  private var running = false
  private val params = WindowManager.LayoutParams().apply {
    type = WindowManager.LayoutParams.TYPE_APPLICATION
    // Never takes focus or keyboard; touches outside it go to the windows below
    flags = WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE or WindowManager.LayoutParams.FLAG_NOT_TOUCH_MODAL or
      WindowManager.LayoutParams.FLAG_LAYOUT_IN_SCREEN or WindowManager.LayoutParams.FLAG_LAYOUT_NO_LIMITS or
      WindowManager.LayoutParams.FLAG_HARDWARE_ACCELERATED
    format = PixelFormat.TRANSLUCENT
    gravity = Gravity.TOP or Gravity.START
    // No window animation, so a raise swaps in place
    windowAnimations = 0
    title = "AcpiraPerfHud"
    layoutInDisplayCutoutMode = WindowManager.LayoutParams.LAYOUT_IN_DISPLAY_CUTOUT_MODE_SHORT_EDGES
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) setFitInsetsTypes(0)
  }

  /** Where the panel sits in the free room of the safe area (0…1 on each axis); it starts at the top end */
  private var anchorX = 1f
  private var anchorY = 0f
  private var placedW = 0
  private var placedH = 0
  private var dragFromX = 0
  private var dragFromY = 0

  private var last = 0L
  private var lastPaint = 0L
  private var expected = 1_000_000_000L / 60
  /** Recent frame intervals (ns) and whether each was a hitch, a ring buffer for the graph */
  private val recent = LongArray(HudLayout.BARS)
  private val recentHitch = BooleanArray(HudLayout.BARS)
  private var recentCount = 0
  private var recentHead = 0
  /** Frame times within the last second, for the rate; sized for a 240 Hz panel */
  private val stamps = LongArray(HudLayout.STAMPS)
  private var stampCount = 0
  private var stampHead = 0
  /** Hitches since the last clear, and each one's time and lateness (ns) within the ratio's window */
  private var hitchCount = 0
  private val hitchWindow = ArrayDeque<LongArray>()
  private var lastHitchAt = 0L
  private var lastHitchLate = 0L

  override fun onCreate(owner: LifecycleOwner) {
    huds += this
    prefs.registerOnSharedPreferenceChangeListener(prefsListener)
  }

  override fun onStart(owner: LifecycleOwner) {
    started = true
    // Frames stopped in the background; the gap before the next one is no hitch
    last = 0
    decor.viewTreeObserver.addOnWindowFocusChangeListener(focusListener)
    sync()
  }

  override fun onStop(owner: LifecycleOwner) {
    started = false
    decor.viewTreeObserver.removeOnWindowFocusChangeListener(focusListener)
    stop()
  }

  override fun onDestroy(owner: LifecycleOwner) {
    huds -= this
    prefs.unregisterOnSharedPreferenceChangeListener(prefsListener)
    stop()
  }

  private fun sync() {
    val fps = isOn(prefs, FPS)
    val hitches = isOn(prefs, HITCHES)
    val changed = fps != showFps || hitches != showHitches
    showFps = fps
    showHitches = hitches
    if (started && (fps || hitches)) { if (changed || !added) start() } else stop()
  }

  private fun start() {
    val v = view ?: HudView(activity, this).also { view = it }
    v.mode(showFps, showHitches)
    params.width = v.fitWidth
    params.height = v.fitHeight
    place()
    if (!added) {
      wm.addView(v, params)
      added = true
    }
    if (!running) {
      running = true
      post()
    }
    clear()
  }

  private fun stop() {
    if (running) {
      if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) (vsync as? Choreographer.VsyncCallback)?.let { Choreographer.getInstance().removeVsyncCallback(it) }
      Choreographer.getInstance().removeFrameCallback(this)
    }
    running = false
    view?.let { if (added) wm.removeViewImmediate(it) }
    added = false
    view = null
  }

  /** Re-adds the overlay so it is the newest app window again, above whatever just opened */
  private fun raise() {
    val v = view ?: return
    if (!added || !started) return
    wm.removeViewImmediate(v)
    wm.addView(v, params)
  }

  internal fun clear() {
    recentCount = 0
    stampCount = 0
    hitchWindow.clear()
    hitchCount = 0
    lastHitchAt = 0
    last = 0
    paint(System.nanoTime())
  }

  /** Android 13 and later: the frame's data, its timelines included */
  private val vsync: Any? = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) Vsync() else null

  @RequiresApi(Build.VERSION_CODES.TIRAMISU)
  private inner class Vsync : Choreographer.VsyncCallback {
    override fun onVsync(data: Choreographer.FrameData) {
      if (!running) return
      Choreographer.getInstance().postVsyncCallback(this)
      // Successive timelines lie one of the app's frames apart
      val lines = data.frameTimelines
      if (lines.size > 1) measured = lines[1].expectedPresentationTimeNanos - lines[0].expectedPresentationTimeNanos
      frame(data.frameTimeNanos)
    }
  }

  /** The frame interval read off the last frame's timelines (ns); 0 until one came */
  private var measured = 0L

  private fun post() {
    val v = vsync
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU && v is Choreographer.VsyncCallback) Choreographer.getInstance().postVsyncCallback(v)
    else Choreographer.getInstance().postFrameCallback(this)
  }

  override fun doFrame(frameTimeNanos: Long) {
    if (!running) return
    Choreographer.getInstance().postFrameCallback(this)
    frame(frameTimeNanos)
  }

  private fun frame(frameTimeNanos: Long) {
    val now = frameTimeNanos
    if (measured > 0) expected = measured.coerceAtLeast(HudLayout.MIN_INTERVAL)
    // The rate: frame times within the last second
    stamps[(stampHead + stampCount) % HudLayout.STAMPS] = now
    if (stampCount < HudLayout.STAMPS) stampCount++ else stampHead = (stampHead + 1) % HudLayout.STAMPS
    while (stampCount > 0 && now - stamps[stampHead] > HudLayout.SECOND) {
      stampHead = (stampHead + 1) % HudLayout.STAMPS
      stampCount--
    }
    val previous = last
    last = now
    if (previous == 0L) return
    val interval = now - previous
    val hitch = interval > expected * HudLayout.HITCH_FACTOR
    recent[(recentHead + recentCount) % HudLayout.BARS] = interval
    recentHitch[(recentHead + recentCount) % HudLayout.BARS] = hitch
    if (recentCount < HudLayout.BARS) recentCount++ else recentHead = (recentHead + 1) % HudLayout.BARS
    if (hitch) {
      val late = interval - expected
      hitchCount++
      hitchWindow.addLast(longArrayOf(now, late))
      lastHitchAt = now
      lastHitchLate = late
      val ms = (late / 1_000_000.0).roundToInt()
      Log.i(TAG, "hitch $ms ms late")
      // A spike on a counter track at the frame that ended the hitch, back to zero on the next one
      if (Trace.isEnabled()) Trace.setCounter(TAG, ms.toLong())
      view?.flash()
    } else if (lastHitchAt == previous && Trace.isEnabled()) {
      Trace.setCounter(TAG, 0)
    }
    while (hitchWindow.isNotEmpty() && now - hitchWindow.first()[0] > HudLayout.RATIO_WINDOW) hitchWindow.removeFirst()
    if (now - lastPaint >= HudLayout.PAINT_EVERY || hitch) paint(now)
  }

  private fun paint(now: Long) {
    lastPaint = now
    val v = view ?: return
    if (measured == 0L) expected = expectedInterval()
    placeIfResized()
    val span = if (stampCount > 1) stamps[(stampHead + stampCount - 1) % HudLayout.STAMPS] - stamps[stampHead] else 0L
    val fps = if (span > 0) ((stampCount - 1) * 1e9 / span).roundToInt() else 0
    // Hitch time ratio over the window, Apple's measure: under 5 ms/s reads smooth, over 10 ms/s plainly janky
    val ratio = hitchWindow.sumOf { it[1] } / 1e6 / (HudLayout.RATIO_WINDOW / 1e9)
    val lastMs = if (lastHitchAt > 0 && now - lastHitchAt < HudLayout.LAST_HITCH_SHOWN) (lastHitchLate / 1e6).roundToInt() else null
    v.update(fps, (1e9 / expected).roundToInt(), recent, recentHitch, recentHead, recentCount, expected, hitchCount, ratio, lastMs)
  }

  /** One frame at the display's current refresh rate: the expected interval where the frames carry no timelines */
  @Suppress("DEPRECATION")
  private fun expectedInterval(): Long {
    val display = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) activity.display else wm.defaultDisplay
    val rate = display?.refreshRate?.takeIf { it > 0f } ?: 60f
    return (1e9 / rate).toLong()
  }

  /** The room the panel may sit in: the activity's window inside its system bars and cutout, below the top app bar,
   *  less a margin */
  private fun area(): IntArray {
    val d = decor
    val w = if (d.width > 0) d.width else activity.resources.displayMetrics.widthPixels
    val h = if (d.height > 0) d.height else activity.resources.displayMetrics.heightPixels
    val insets = ViewCompat.getRootWindowInsets(d)
      ?.getInsets(WindowInsetsCompat.Type.systemBars() or WindowInsetsCompat.Type.displayCutout())
    val m = px(HudLayout.MARGIN).roundToInt()
    val bar = px(HudLayout.BAR_CLEARANCE).roundToInt()
    return intArrayOf((insets?.left ?: 0) + m, (insets?.top ?: 0) + bar + m, w - (insets?.right ?: 0) - m, h - (insets?.bottom ?: 0) - m)
  }

  /** Puts the panel at its anchor inside the safe area */
  private fun place() {
    val (l, top, r, b) = area()
    placedW = decor.width
    placedH = decor.height
    params.x = l + ((r - l - params.width) * anchorX).roundToInt()
    params.y = top + ((b - top - params.height) * anchorY).roundToInt()
    view?.let { if (added) wm.updateViewLayout(it, params) }
  }

  /** After a rotation or a window resize, checked as the panel repaints */
  private fun placeIfResized() {
    if (decor.width != placedW || decor.height != placedH) place()
  }

  internal fun dragStart() {
    dragFromX = params.x
    dragFromY = params.y
  }

  internal fun dragBy(dx: Float, dy: Float) {
    val v = view ?: return
    params.x = dragFromX + dx.roundToInt()
    params.y = dragFromY + dy.roundToInt()
    if (added) wm.updateViewLayout(v, params)
  }

  /** The drop becomes an anchor, so the panel keeps its corner across rotation */
  internal fun dragEnd() {
    val (l, top, r, b) = area()
    val freeW = (r - l - params.width).coerceAtLeast(1)
    val freeH = (b - top - params.height).coerceAtLeast(1)
    anchorX = ((params.x - l).toFloat() / freeW).coerceIn(0f, 1f)
    anchorY = ((params.y - top).toFloat() / freeH).coerceIn(0f, 1f)
    place()
  }

  private fun px(dp: Float) = dp * activity.resources.displayMetrics.density
}

/** The overlay's geometry, colours and pacing: a debug instrument outside the app's design system, with its own values */
private object HudLayout {
  const val BARS = 90
  const val STAMPS = 256
  const val BAR_WIDTH = 1.5f
  const val GRAPH_HEIGHT = 28f
  /** Frame time at the graph's top (ns); longer frames are clipped there */
  const val GRAPH_CEILING = 50_000_000L
  const val PAD = 6f
  const val GAP = 3f
  const val RADIUS = 8f
  const val BORDER = 1f
  const val FONT = 11f
  const val WEIGHT = 600
  const val MARGIN = 8f
  /** Room left for the top app bar under the status bar, so the panel never covers back or the bar's actions (dp) */
  const val BAR_CLEARANCE = 56f
  const val HITCH_FACTOR = 1.5
  /** The shortest expected frame interval taken from a frame's timelines: a 240 Hz frame (ns) */
  const val MIN_INTERVAL = 1_000_000_000L / 240
  const val SECOND = 1_000_000_000L
  const val PAINT_EVERY = 200_000_000L
  const val RATIO_WINDOW = 5_000_000_000L
  const val LAST_HITCH_SHOWN = 3_000_000_000L
  const val FLASH_MS = 600L
  /** Rate below these fractions of the target turns yellow, then red */
  const val RATE_OK = 0.9
  const val RATE_WARN = 2.0 / 3
  /** Hitch ratio (ms/s) at which the hitch line turns yellow, then red */
  const val RATIO_WARN = 5.0
  const val RATIO_BAD = 10.0

  val BG = Color.argb(184, 0, 0, 0)
  val EDGE = Color.argb(31, 255, 255, 255)
  val BAR = Color.argb(140, 255, 255, 255)
  val TEXT = Color.WHITE
  val RED = Color.rgb(255, 69, 58)
  val YELLOW = Color.rgb(255, 214, 10)
  val BUDGET = Color.argb(179, 48, 209, 88)
}

/**
 * The panel: the rate over the last second with a graph of recent frame times, and the hitch count with the hitch
 * time ratio; a hitch flashes its edge and shows how late it came for a few seconds
 */
@SuppressLint("ViewConstructor")
private class HudView(context: Context, private val hud: PerfHud) : View(context) {
  private val density = resources.displayMetrics.density
  private fun px(dp: Float) = dp * density

  private val textPaint = Paint(Paint.ANTI_ALIAS_FLAG).apply {
    // Semibold with tabular digits, so the numbers hold still as they change
    typeface = Typeface.create(Typeface.DEFAULT, HudLayout.WEIGHT, false)
    fontFeatureSettings = "tnum"
    textSize = TypedValue.applyDimension(TypedValue.COMPLEX_UNIT_SP, HudLayout.FONT, resources.displayMetrics)
  }
  private val fill = Paint(Paint.ANTI_ALIAS_FLAG)
  private val edge = Paint(Paint.ANTI_ALIAS_FLAG).apply { style = Paint.Style.STROKE; strokeWidth = px(HudLayout.BORDER) }
  private val bar = Paint()
  private val budget = Paint().apply { color = HudLayout.BUDGET; strokeWidth = px(HudLayout.BORDER) }
  private val box = RectF()
  private val ascent = -textPaint.fontMetrics.ascent
  private val lineHeight = ceil(ascent + textPaint.fontMetrics.descent).toInt()

  private var fps = true
  private var hitches = true
  private var rateText = ""
  private var rateColor = HudLayout.TEXT
  private var hitchText = ""
  private var hitchColor = HudLayout.TEXT
  private val frames = LongArray(HudLayout.BARS)
  private val late = BooleanArray(HudLayout.BARS)
  private var frameCount = 0
  private var expected = 0L
  private var flashAt = 0L

  private val slop = ViewConfiguration.get(context).scaledTouchSlop
  private var downX = 0f
  private var downY = 0f
  private var dragging = false

  init {
    importantForAccessibility = IMPORTANT_FOR_ACCESSIBILITY_NO
  }

  private val graphWidth get() = px(HudLayout.BARS * HudLayout.BAR_WIDTH)

  val fitWidth get() = (graphWidth + px(HudLayout.PAD) * 2).roundToInt()
  val fitHeight: Int get() {
    val parts = buildList {
      if (fps) { add(lineHeight.toFloat()); add(px(HudLayout.GRAPH_HEIGHT)) }
      if (hitches) add(lineHeight.toFloat())
    }
    return (px(HudLayout.PAD) * 2 + parts.sum() + px(HudLayout.GAP) * (parts.size - 1).coerceAtLeast(0)).roundToInt()
  }

  fun mode(fps: Boolean, hitches: Boolean) {
    this.fps = fps
    this.hitches = hitches
    invalidate()
  }

  fun update(
    fps: Int, target: Int, ring: LongArray, ringHitch: BooleanArray, head: Int, count: Int, expected: Long,
    hitchCount: Int, ratio: Double, lastHitchMs: Int?,
  ) {
    if (this.fps) {
      rateText = "$fps / $target FPS"
      rateColor = when {
        fps >= target * HudLayout.RATE_OK -> HudLayout.TEXT
        fps >= target * HudLayout.RATE_WARN -> HudLayout.YELLOW
        else -> HudLayout.RED
      }
      // A snapshot, oldest first: the ring moves on before this draws
      for (i in 0 until count) {
        frames[i] = ring[(head + i) % HudLayout.BARS]
        late[i] = ringHitch[(head + i) % HudLayout.BARS]
      }
      frameCount = count
      this.expected = expected
    }
    if (hitches) {
      val text = StringBuilder("${t("mobile.dev.hud.hitches")} $hitchCount  ${"%.1f".format(ratio)} ms/s")
      if (lastHitchMs != null) text.append("  +${lastHitchMs}ms")
      hitchText = text.toString()
      hitchColor = when {
        ratio < HudLayout.RATIO_WARN -> HudLayout.TEXT
        ratio < HudLayout.RATIO_BAD -> HudLayout.YELLOW
        else -> HudLayout.RED
      }
    }
    invalidate()
  }

  /** A hitch's mark on the edge, fading back */
  fun flash() {
    flashAt = SystemClock.uptimeMillis()
    invalidate()
  }

  override fun onDraw(canvas: Canvas) {
    val half = edge.strokeWidth / 2
    box.set(half, half, width - half, height - half)
    val r = px(HudLayout.RADIUS)
    fill.color = HudLayout.BG
    canvas.drawRoundRect(box, r, r, fill)
    // The edge fades from red back to its hairline over the flash
    val since = SystemClock.uptimeMillis() - flashAt
    edge.color = if (since < HudLayout.FLASH_MS) {
      postInvalidateOnAnimation()
      ColorUtils.blendARGB(HudLayout.RED, HudLayout.EDGE, since.toFloat() / HudLayout.FLASH_MS)
    } else HudLayout.EDGE
    canvas.drawRoundRect(box, r, r, edge)

    val pad = px(HudLayout.PAD)
    var y = pad
    if (fps) {
      textPaint.color = rateColor
      canvas.drawText(rateText, pad, y + ascent, textPaint)
      y += lineHeight + px(HudLayout.GAP)
      val h = px(HudLayout.GRAPH_HEIGHT)
      val w = px(HudLayout.BAR_WIDTH)
      // Right-aligned, newest at the right edge
      val start = pad + (HudLayout.BARS - frameCount) * w
      for (i in 0 until frameCount) {
        val b = (frames[i].toFloat() / HudLayout.GRAPH_CEILING).coerceAtMost(1f) * h
        bar.color = if (late[i]) HudLayout.RED else HudLayout.BAR
        canvas.drawRect(start + i * w, y + h - b, start + (i + 1) * w, y + h, bar)
      }
      // The frame budget: bars above this line came late
      if (expected > 0) {
        val line = y + h - (expected.toFloat() / HudLayout.GRAPH_CEILING).coerceAtMost(1f) * h
        canvas.drawLine(pad, line, pad + graphWidth, line, budget)
      }
      y += h + px(HudLayout.GAP)
    }
    if (hitches) {
      textPaint.color = hitchColor
      canvas.drawText(hitchText, pad, y + ascent, textPaint)
    }
  }

  @SuppressLint("ClickableViewAccessibility")
  override fun onTouchEvent(event: MotionEvent): Boolean {
    when (event.actionMasked) {
      MotionEvent.ACTION_DOWN -> {
        downX = event.rawX
        downY = event.rawY
        dragging = false
        hud.dragStart()
      }
      MotionEvent.ACTION_MOVE -> {
        val dx = event.rawX - downX
        val dy = event.rawY - downY
        if (!dragging && (abs(dx) > slop || abs(dy) > slop)) dragging = true
        if (dragging) hud.dragBy(dx, dy)
      }
      MotionEvent.ACTION_UP -> if (dragging) hud.dragEnd() else hud.clear()
      MotionEvent.ACTION_CANCEL -> if (dragging) hud.dragEnd()
    }
    return true
  }
}
