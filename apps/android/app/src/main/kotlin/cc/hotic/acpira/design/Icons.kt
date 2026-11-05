package cc.hotic.acpira.design

import android.content.Context
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.PathFillType
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.drawscope.scale
import androidx.compose.ui.graphics.drawscope.translate
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.PathParser
import androidx.compose.ui.graphics.vector.path
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json

// The lucide icons the webview imports and the vendor marks, read from assets/icons.json (scripts/export/android-icons.ts
// writes it from lucide-react and chat/marks.tsx): the same geometry the webview and the iOS app draw.

@Serializable
private data class IconSet(val lucide: Map<String, List<String>>, val marks: Marks, val brands: List<List<String>> = emptyList()) {
  @Serializable
  data class Marks(val paths: Map<String, List<String>>, val alias: Map<String, String>)
}

object IconStore {
  @Volatile private var set: IconSet? = null
  private val vectors = HashMap<String, ImageVector>()
  private val marks = HashMap<String, Pair<Path, Rect>?>()
  @Volatile private var brandRules: List<Pair<Regex, String>>? = null

  private fun load(context: Context): IconSet = set ?: synchronized(this) {
    set ?: context.assets.open("icons.json").bufferedReader().use { r ->
      Json { ignoreUnknownKeys = true }.decodeFromString(IconSet.serializer(), r.readText())
    }.also { set = it }
  }

  /** A lucide icon as a stroked 24-unit vector (round caps and joins, stroke 2, as lucide-react draws it) */
  fun lucide(context: Context, name: String): ImageVector? = synchronized(vectors) {
    vectors[name] ?: run {
      val data = load(context).lucide[name] ?: return@synchronized null
      ImageVector.Builder(name, 24.dp, 24.dp, 24f, 24f).apply {
        for (d in data) {
          addPath(
            pathData = PathParser().parsePathString(d).toNodes(),
            stroke = SolidColor(Color.Black), strokeLineWidth = 2f,
            strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round,
          )
        }
      }.build().also { vectors[name] = it }
    }
  }

  private val elements = HashMap<String, List<Path>>()

  /** A lucide icon's elements as separate 24-unit paths, in source order (for glyphs drawn or animated by hand) */
  fun lucidePaths(context: Context, name: String): List<Path>? = synchronized(elements) {
    elements[name] ?: run {
      val data = load(context).lucide[name] ?: return@synchronized null
      data.map { PathParser().parsePathString(it).toPath() }.also { elements[name] = it }
    }
  }

  /** The brand key of a model name or id ("Claude Fable 5.1" → claude), by shared/models.ts BRAND, first match wins */
  fun brandOf(context: Context, model: String): String? {
    val rules = brandRules ?: load(context).brands.mapNotNull { r -> r.getOrNull(1)?.let { Regex(r[0]) to it } }.also { brandRules = it }
    val lower = model.lowercase()
    return rules.firstOrNull { it.first.containsMatchIn(lower) }?.second
  }

  /** Marks the engine sent with its agents (AgentInfo.mark: plugin and agents.json agents), by agent id */
  private val adopted = HashMap<String, List<String>>()

  /** Bumped when [adopt] changes a mark, so a VendorMark drawn before the agents arrived draws again */
  var marksVersion by mutableIntStateOf(0)
    private set

  /** Takes the agents' own marks; a built-in brand of the same id still wins in [mark] */
  fun adopt(byId: Map<String, List<String>>) {
    val changed = synchronized(marks) {
      val ids = (adopted.keys + byId.keys).filter { adopted[it] != byId[it] }
      adopted.clear()
      adopted.putAll(byId)
      ids.forEach { marks.remove(it) }
      ids.isNotEmpty()
    }
    if (changed) marksVersion++
  }

  /** A vendor mark's filled path (even-odd) and the bounds of its ink, null for an unknown brand */
  fun mark(context: Context, brand: String): Pair<Path, Rect>? = synchronized(marks) {
    if (marks.containsKey(brand)) return@synchronized marks[brand]
    val icons = load(context).marks
    val data = icons.paths[icons.alias[brand] ?: brand] ?: adopted[brand]
    val made = data?.let { ds ->
      val path = Path().apply { fillType = PathFillType.EvenOdd }
      for (d in ds) path.addPath(PathParser().parsePathString(expandArcFlags(d)).toPath())
      path to path.getBounds()
    }
    marks[brand] = made
    made
  }
}

private val svgNumber = Regex("""[-+]?(?:\d+\.?\d*|\.\d+)(?:[eE][-+]?\d+)?""")

/**
 * Spells out the arc flags SVG lets run together with what follows (`a14 14 0 01-4.45-3`, as the lobehub marks are
 * minified): PathParser reads `01` as one number, which breaks the Gemini mark
 */
internal fun expandArcFlags(d: String): String {
  if (d.none { it == 'a' || it == 'A' }) return d
  val out = StringBuilder(d.length + 16)
  var i = 0
  var command = ' '
  fun skip() {
    while (i < d.length && (d[i] == ' ' || d[i] == ',' || d[i].isWhitespace())) i++
  }
  fun number() {
    val m = svgNumber.find(d, i)?.takeIf { it.range.first == i } ?: error("bad path at $i")
    out.append(' ').append(m.value)
    i = m.range.last + 1
  }
  while (true) {
    skip()
    if (i >= d.length) break
    val c = d[i]
    if (c.isLetter()) {
      command = c
      out.append(c)
      i++
    } else if (command == 'a' || command == 'A') {
      // rx ry rotation large-arc sweep x y: the two flags are one character each
      for (p in 0 until 7) {
        skip()
        if (p == 3 || p == 4) out.append(' ').append(d[i++]) else number()
      }
    } else {
      number()
    }
  }
  return out.toString()
}

/** A lucide icon by its camelCase name (`chevronRight`), tinted like text */
@Composable
fun Lucide(name: String, tint: Color, modifier: Modifier = Modifier, size: Dp = Metrics.icon, contentDescription: String? = null) {
  val context = LocalContext.current
  val vector = remember(name) { IconStore.lucide(context, name) }
  if (vector != null) Icon(vector, contentDescription, modifier.size(size), tint = tint)
  else Box(modifier.size(size))
}

/**
 * chat/marks.tsx MarkSvg: a vendor logo filled in [tint], or the name's initial in a rounded square when the brand is
 * unknown, so rows with and without a known brand stay aligned. [fit]: the logo's own ink fills the frame, centred,
 * instead of its 24-unit box (logos sit off-centre inside their boxes, which shows in one column)
 */
@Composable
fun VendorMark(brand: String?, name: String, tint: Color, modifier: Modifier = Modifier, size: Dp = Metrics.icon, fit: Boolean = false) {
  val context = LocalContext.current
  val mark = remember(brand, IconStore.marksVersion) { brand?.let { IconStore.mark(context, it) } }
  if (mark != null) {
    val (path, bounds) = mark
    Canvas(modifier.size(size)) {
      val box = if (fit) bounds else Rect(0f, 0f, 24f, 24f)
      val k = minOf(this.size.width / box.width, this.size.height / box.height)
      val dx = (this.size.width - box.width * k) / 2 - box.left * k
      val dy = (this.size.height - box.height * k) / 2 - box.top * k
      translate(dx, dy) { scale(k, k, pivot = androidx.compose.ui.geometry.Offset.Zero) { drawPath(path, tint) } }
    }
  } else {
    val letter = name.trim().firstOrNull()?.uppercase() ?: "?"
    Box(
      modifier.size(size).background(tint.copy(alpha = 0.14f), RoundedCornerShape(size * 0.28f)),
      contentAlignment = Alignment.Center,
    ) {
      Text(letter, color = tint, fontSize = (size.value * 0.58f).sp, fontWeight = FontWeight.SemiBold)
    }
  }
}
