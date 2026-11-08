package cc.hotic.acpira.logic

import com.google.zxing.EncodeHintType
import com.google.zxing.common.BitMatrix
import com.google.zxing.qrcode.decoder.ErrorCorrectionLevel
import com.google.zxing.qrcode.encoder.Encoder
import kotlin.math.hypot
import kotlin.math.roundToInt

// The code the phone pairs with, as the computer's pairing card draws it (apps/ios Pairing/PairCode.swift,
// packages/ui/remote/pairGeometry.ts): round modules in rings from the centre, rounded finder eyes, Acpira's mark in a
// knock-out. The pure part: the modules as rows, and which ring each piece belongs to; ui/PairCode.kt draws them.

/** A code's modules as rows of `1` (dark) and `0`, no quiet zone, as the gateway sends them to its card */
object PairCodeRows {
  /**
   * The symbol a scan read (ZXing's sampled grid): the card's very modules, its mask included. Without one (a pasted
   * link, a link opened from outside) a code made from the link's text ([make]), which may pick another mask
   */
  fun of(bits: BitMatrix): List<String>? {
    val n = bits.width
    if (n == 0 || bits.height != n) return null
    return (0 until n).map { y -> buildString(n) { for (x in 0 until n) append(if (bits[x, y]) '1' else '0') } }
  }

  /** A code made from [link]'s text at the gateway's `qrcode` default correction (M) */
  fun make(link: String): List<String>? = runCatching {
    val matrix = Encoder.encode(link, ErrorCorrectionLevel.M, mapOf(EncodeHintType.CHARACTER_SET to "UTF-8")).matrix
    val n = matrix.width
    if (n == 0 || matrix.height != n) return null
    (0 until n).map { y -> buildString(n) { for (x in 0 until n) append(if (matrix[x, y].toInt() == 1) '1' else '0') } }
  }.getOrNull()
}

/**
 * The code's pieces in module units, sorted into rings from the centre out (pairGeometry.ts `qrShape`): [dots] holds
 * each ring's dark modules (x, y of the module's corner), [eyes] each ring's finder eyes (their 7x7 box's corner).
 * Finder modules and the centre knock-out are left out of [dots]
 */
class PairCodeShape(rows: List<String>) {
  val n: Int = rows.size
  /** The centre knock-out for the mark, in modules */
  val hole: Int = holeOf(n)
  val dots: List<List<Pair<Int, Int>>>
  val eyes: List<List<Pair<Int, Int>>>

  init {
    val c = (n - 1) / 2.0
    val far = maxOf(hypot(c, c), 1.0)
    fun band(x: Int, y: Int) = minOf(BANDS - 1, (hypot(x - c, y - c) / far * BANDS).toInt())
    fun finder(x: Int, y: Int) = (x < EYE && y < EYE) || (x >= n - EYE && y < EYE) || (x < EYE && y >= n - EYE)
    val h0 = (n - hole) / 2
    fun open(x: Int, y: Int) = x >= h0 && x < h0 + hole && y >= h0 && y < h0 + hole
    val d = List(BANDS) { ArrayList<Pair<Int, Int>>() }
    val e = List(BANDS) { ArrayList<Pair<Int, Int>>() }
    rows.forEachIndexed { y, row ->
      row.forEachIndexed { x, ch -> if (ch == '1' && !finder(x, y) && !open(x, y)) d[band(x, y)] += x to y }
    }
    if (n >= EYE) for ((fx, fy) in listOf(0 to 0, n - EYE to 0, 0 to n - EYE)) e[band(fx + EYE / 2, fy + EYE / 2)] += fx to fy
    dots = d
    eyes = e
  }

  companion object {
    /** The quiet zone in modules, and the rings the paired moment dissolves outward */
    const val QUIET = 4
    const val BANDS = 11
    /** A finder eye's side in modules */
    const val EYE = 7
    /** A dot's radius and the eye's three corner radii (outer ring, its hole, the inner square), in modules */
    const val DOT = 0.42f
    val EYE_RADII = floatArrayOf(2.3f, 1.5f, 1.05f)

    /** The knock-out: 11 on a 57-module code, smaller on smaller codes, odd so it is centred on whole modules */
    fun holeOf(n: Int): Int {
      val h = maxOf(7, minOf(11, (n * 11.0 / 57).roundToInt()))
      return if (h % 2 == n % 2) h else h - 1
    }
  }
}
