package cc.hotic.acpira.design

import android.content.res.AssetManager
import android.graphics.Typeface

// The KaTeX faces and their TeX metrics for the math typesetter, ported from apps/ios AcpiraDesign/KaTeXFont.swift.
// The faces are the iOS package's own TTFs and the metric tables its generated KaTeXMetrics.generated.swift, both
// synced into the assets by `syncSharedAssets` (apps/android/app/build.gradle.kts); the Swift source is parsed here
// at runtime so there stays one export script.

/** A bundled KaTeX face; `rawValue` is the iOS enum's raw value and the key of its metric table */
enum class KaTeXFace(val rawValue: String) {
  Main("Main-Regular"),
  MainBold("Main-Bold"),
  Math("Math-Italic"),
  Size1("Size1-Regular"),
  Size2("Size2-Regular"),
  Ams("AMS-Regular");

  /** Where `syncSharedAssets` puts the TTF */
  val assetPath: String get() = "fonts/KaTeX_$rawValue.ttf"
}

/** One glyph's TeX metrics in em of its face */
data class TeXGlyphMetrics(val depth: Double, val height: Double, val italic: Double, val skew: Double, val width: Double)

/** The loaded faces and tables; get one through [KaTeXFont.get] */
class KaTeXFont private constructor(
  private val typefaces: Map<KaTeXFace, Typeface>,
  private val tables: Map<KaTeXFace, Map<Int, TeXGlyphMetrics>>,
  private val params: Map<String, DoubleArray>,
) {
  /** The face's typeface; Typeface.DEFAULT when its asset failed to load */
  fun typeface(face: KaTeXFace): Typeface = typefaces[face] ?: Typeface.DEFAULT

  /** A code point's metrics in a face, null when the face has no such glyph */
  fun metrics(face: KaTeXFace, codePoint: Int): TeXGlyphMetrics? = tables[face]?.get(codePoint)

  /** A TeX font parameter for a size index (0 text, 1 script, 2 scriptscript), in em of that size */
  fun param(name: String, sizeIndex: Int): Double = params[name]?.getOrNull(sizeIndex.coerceIn(0, 2)) ?: 0.0

  companion object {
    private const val METRICS_ASSET = "katex/KaTeXMetrics.generated.swift"

    /** `"Face": "code:d,h,i,s,w;…",` lines of `packed` */
    private val packedLine = Regex("""^\s*"([A-Za-z0-9-]+)":\s*"([^"]*)",?\s*$""")

    /** `"name": [a, b, c],` lines of `params` */
    private val paramLine = Regex("""^\s*"(\w+)":\s*\[([^\]]*)\],?\s*$""")

    @Volatile private var shared: KaTeXFont? = null

    /** The faces and tables, loaded once on first use; safe from any thread */
    fun get(assets: AssetManager): KaTeXFont = shared ?: synchronized(this) {
      shared ?: load(assets).also { shared = it }
    }

    private fun load(assets: AssetManager): KaTeXFont {
      val typefaces = KaTeXFace.entries.mapNotNull { face ->
        runCatching { face to Typeface.createFromAsset(assets, face.assetPath) }.getOrNull()
      }.toMap()
      val source = runCatching { assets.open(METRICS_ASSET).bufferedReader().use { it.readText() } }.getOrDefault("")
      val (tables, params) = parse(source)
      return KaTeXFont(typefaces, tables, params)
    }

    /** Reads the packed glyph tables and the parameter dictionary out of the generated Swift source */
    internal fun parse(source: String): Pair<Map<KaTeXFace, Map<Int, TeXGlyphMetrics>>, Map<String, DoubleArray>> {
      val faces = KaTeXFace.entries.associateBy { it.rawValue }
      val tables = HashMap<KaTeXFace, Map<Int, TeXGlyphMetrics>>()
      val params = HashMap<String, DoubleArray>()
      for (line in source.lineSequence()) {
        val packed = packedLine.matchEntire(line)
        if (packed != null) {
          faces[packed.groupValues[1]]?.let { tables[it] = parseTable(packed.groupValues[2]) }
          continue
        }
        val param = paramLine.matchEntire(line) ?: continue
        val values = param.groupValues[2].split(',').mapNotNull { it.trim().toDoubleOrNull() }
        if (values.isNotEmpty()) params[param.groupValues[1]] = values.toDoubleArray()
      }
      return tables to params
    }

    private fun parseTable(packed: String): Map<Int, TeXGlyphMetrics> {
      val glyphs = HashMap<Int, TeXGlyphMetrics>()
      for (entry in packed.split(';')) {
        val parts = entry.split(':')
        if (parts.size != 2) continue
        val code = parts[0].toIntOrNull() ?: continue
        val v = parts[1].split(',').mapNotNull { it.toDoubleOrNull() }
        if (v.size != 5) continue
        glyphs[code] = TeXGlyphMetrics(depth = v[0], height = v[1], italic = v[2], skew = v[3], width = v[4])
      }
      return glyphs
    }
  }
}
