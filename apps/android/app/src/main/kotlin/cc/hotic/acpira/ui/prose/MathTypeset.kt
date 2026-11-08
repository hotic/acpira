package cc.hotic.acpira.ui.prose

import android.graphics.Paint
import android.graphics.Typeface
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.asComposePath
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.clearAndSetSemantics
import cc.hotic.acpira.design.KaTeXFace
import cc.hotic.acpira.design.KaTeXFont
import cc.hotic.acpira.design.MathTokens
import cc.hotic.acpira.design.Metrics
import cc.hotic.acpira.design.Palette
import cc.hotic.acpira.design.WebType
import kotlin.math.ceil
import kotlin.math.max
import kotlin.math.min

// `$$` display math, typeset the way KaTeX does it for the webview, ported from apps/ios
// AcpiraUI/Prose/MathTypeset.swift: the same faces (bundled KaTeX fonts), the same TFM metrics (KaTeXFont) and the
// same placement rules for the TeX subset agents write — symbols with TeX inter-atom spacing, sub- and superscripts,
// big operators with limits, fractions, roots, \left…\right delimiters, accents, \text and operator names. Everything
// is laid out in pixels, y up from the baseline, and drawn as glyph outlines, so each glyph lands where KaTeX's CSS
// boxes put it.

internal enum class MathClass { Ord, Op, Bin, Rel, Open, Close, Punct, Inner }

/** One atom of the parsed formula; symbols are single code points held as strings */
internal sealed interface MathNode {
  /** One symbol in a face */
  data class Symbol(val c: String, val face: KaTeXFace, val cls: MathClass) : MathNode
  /** Upright text in a face (\text, \mathrm, operator names) */
  data class Word(val text: String, val face: KaTeXFace, val cls: MathClass) : MathNode
  data class Row(val nodes: List<MathNode>) : MathNode
  data class Scripts(val base: MathNode?, val sup: MathNode?, val sub: MathNode?) : MathNode
  /** A large operator (∑ ∏ ∫ …): Size2 in display style, Size1 otherwise; `limits` puts scripts above and below */
  data class BigOp(val c: String, val limits: Boolean) : MathNode
  /** A named operator (\lim, \sin …) in the upright face */
  data class NamedOp(val name: String, val limits: Boolean) : MathNode
  data class Frac(val num: MathNode, val den: MathNode) : MathNode
  data class Sqrt(val inner: MathNode) : MathNode
  /** An explicit space in em */
  data class Space(val em: Double) : MathNode
  data class Delimited(val open: String?, val inner: MathNode, val close: String?) : MathNode
  data class Accent(val mark: String, val inner: MathNode) : MathNode
  data class Overline(val inner: MathNode) : MathNode
}

private fun cpString(cp: Int): String = String(Character.toChars(cp))

/** A string's code points, each as its own string */
private fun String.codePointStrings(): List<String> = codePoints().toArray().map(::cpString)

/** Swift's Character.isLetter: the Unicode Alphabetic property */
private fun isLetter(cp: Int) = Character.isAlphabetic(cp)

private fun isWhitespace(cp: Int) = Character.isWhitespace(cp) || Character.isSpaceChar(cp)

/** Recursive descent over the TeX source; anything it does not know falls back to its literal characters */
internal class TeXParser private constructor(source: String) {
  private val chars: IntArray = source.codePoints().toArray()
  private var at = 0

  companion object {
    private const val END = -1

    fun parse(source: String): MathNode = MathNode.Row(TeXParser(source).row())
  }

  private val current: Int get() = if (at < chars.size) chars[at] else END

  private fun skipSpaces() {
    while (current != END && isWhitespace(current)) at++
  }

  /** Atoms until a closing brace, `\right` or the end */
  fun row(): List<MathNode> {
    val out = mutableListOf<MathNode>()
    while (true) {
      skipSpaces()
      val c = current
      if (c == END || c == '}'.code) break
      if (peekCommand() == "right") break
      if (c == '^'.code || c == '_'.code) {
        // A script with no base attaches to an empty one
        at++
        val arg = argument()
        out += if (c == '^'.code) MathNode.Scripts(null, arg, null) else MathNode.Scripts(null, null, arg)
        continue
      }
      val base = primary() ?: continue
      out += scripted(base)
    }
    return out
  }

  /** A base followed by any `^` / `_` / `'` scripts */
  private fun scripted(base: MathNode): MathNode {
    val sup = mutableListOf<MathNode>()
    var sub: MathNode? = null
    var hasSup = false
    while (true) {
      skipSpaces()
      when (current) {
        '\''.code -> {
          at++
          sup += MathNode.Symbol("′", KaTeXFace.Main, MathClass.Ord)
          hasSup = true
        }
        '^'.code -> {
          at++
          argument()?.let { sup += it }
          hasSup = true
        }
        '_'.code -> {
          at++
          sub = argument()
        }
        else -> break
      }
    }
    if (!hasSup && sub == null) return base
    return MathNode.Scripts(base, if (hasSup) MathNode.Row(sup) else null, sub)
  }

  /** A braced group or a single token */
  private fun argument(): MathNode? {
    skipSpaces()
    if (current == '{'.code) {
      at++
      val inner = row()
      if (current == '}'.code) at++
      return MathNode.Row(inner)
    }
    return primary()
  }

  /** The raw text of a braced argument (\text{…}, \operatorname{…}) */
  private fun rawArgument(): String {
    skipSpaces()
    if (current != '{'.code) {
      if (current == END) return ""
      return cpString(chars[at++])
    }
    at++
    var depth = 1
    val out = StringBuilder()
    while (current != END) {
      val c = chars[at++]
      if (c == '{'.code) {
        depth++
      } else if (c == '}'.code) {
        depth--
        if (depth == 0) break
      }
      out.appendCodePoint(c)
    }
    return out.toString()
  }

  private fun peekCommand(): String? {
    if (current != '\\'.code) return null
    var j = at + 1
    val name = StringBuilder()
    while (j < chars.size && isLetter(chars[j])) name.appendCodePoint(chars[j++])
    return name.toString()
  }

  private fun command(): String {
    at++ // backslash
    val c = current
    if (c == END) return ""
    if (!isLetter(c)) {
      at++
      return cpString(c)
    }
    val name = StringBuilder()
    while (current != END && isLetter(current)) name.appendCodePoint(chars[at++])
    return name.toString()
  }

  /** A delimiter after \left / \right: a character or a command naming one; "." is none */
  private fun delimiter(): String? {
    skipSpaces()
    val c = current
    if (c == END) return null
    if (c == '\\'.code) {
      val name = command()
      return TeX.delimiters[name] ?: name.takeIf { it.isNotEmpty() }?.let { cpString(it.codePointAt(0)) }
    }
    at++
    return if (c == '.'.code) null else cpString(c)
  }

  private fun primary(): MathNode? {
    skipSpaces()
    val c = current
    if (c == END) return null
    if (c == '{'.code) {
      at++
      val inner = row()
      if (current == '}'.code) at++
      return MathNode.Row(inner)
    }
    if (c == '\\'.code) return commandNode(command())
    at++
    return TeX.symbol(c)
  }

  private fun commandNode(name: String): MathNode? {
    TeX.spaces[name]?.let { return MathNode.Space(it) }
    TeX.commands[name]?.let { return MathNode.Symbol(it.c, it.face, it.cls) }
    TeX.bigOps[name]?.let { return MathNode.BigOp(it.first, it.second) }
    TeX.namedOps[name]?.let { return MathNode.NamedOp(name, it) }
    TeX.accents[name]?.let { mark -> return argument()?.let { MathNode.Accent(mark, it) } }
    return when (name) {
      "frac", "dfrac", "tfrac", "cfrac" -> {
        val num = argument() ?: MathNode.Row(emptyList())
        MathNode.Frac(num, argument() ?: MathNode.Row(emptyList()))
      }
      "sqrt" -> {
        skipSpaces()
        // The optional index is dropped
        if (current == '['.code) {
          while (current != END && current != ']'.code) at++
          at++
        }
        argument()?.let { MathNode.Sqrt(it) }
      }
      "text", "textrm", "mathrm", "textnormal", "mbox" -> MathNode.Word(rawArgument(), KaTeXFace.Main, MathClass.Ord)
      "textbf", "mathbf", "boldsymbol", "bm" -> MathNode.Word(rawArgument(), KaTeXFace.MainBold, MathClass.Ord)
      "mathit", "textit" -> MathNode.Row(rawArgument().codePointStrings().map { MathNode.Symbol(it, KaTeXFace.Math, MathClass.Ord) })
      "operatorname" -> MathNode.NamedOp(rawArgument(), false)
      "overline" -> argument()?.let { MathNode.Overline(it) }
      "left" -> {
        val open = delimiter()
        val inner = row()
        var close: String? = null
        if (peekCommand() == "right") {
          command()
          close = delimiter()
        }
        MathNode.Delimited(open, MathNode.Row(inner), close)
      }
      // Faces this subset does not carry render the plain letters; style switches are ignored
      "mathbb", "mathcal", "mathfrak", "mathscr", "mathsf", "mathtt", "displaystyle", "textstyle", "limits", "nolimits" ->
        if (name.startsWith("math")) argument() else null
      // Unknown: the command's name, upright, so nothing silently disappears
      else -> MathNode.Word(name, KaTeXFace.Main, MathClass.Ord)
    }
  }
}

/** Symbol tables (KaTeX's symbols.js, the common part) */
internal object TeX {
  class Command(val c: String, val face: KaTeXFace, val cls: MathClass)

  fun symbol(c: Int): MathNode {
    if (c < 128 && Character.isLetter(c)) return MathNode.Symbol(cpString(c), KaTeXFace.Math, MathClass.Ord)
    if (c in '0'.code..'9'.code) return MathNode.Symbol(cpString(c), KaTeXFace.Main, MathClass.Ord)
    return when (c) {
      '+'.code -> MathNode.Symbol("+", KaTeXFace.Main, MathClass.Bin)
      '-'.code -> MathNode.Symbol("−", KaTeXFace.Main, MathClass.Bin)
      '*'.code -> MathNode.Symbol("∗", KaTeXFace.Main, MathClass.Bin)
      '='.code, '<'.code, '>'.code, ':'.code -> MathNode.Symbol(cpString(c), KaTeXFace.Main, MathClass.Rel)
      '('.code, '['.code -> MathNode.Symbol(cpString(c), KaTeXFace.Main, MathClass.Open)
      ')'.code, ']'.code -> MathNode.Symbol(cpString(c), KaTeXFace.Main, MathClass.Close)
      ','.code, ';'.code -> MathNode.Symbol(cpString(c), KaTeXFace.Main, MathClass.Punct)
      '|'.code -> MathNode.Symbol("∣", KaTeXFace.Main, MathClass.Ord)
      '~'.code -> MathNode.Space(0.25)
      else -> MathNode.Symbol(cpString(c), KaTeXFace.Main, MathClass.Ord)
    }
  }

  /** em for \, \: \; \! \quad … */
  val spaces: Map<String, Double> = mapOf(
    "," to 3.0 / 18, ":" to 4.0 / 18, ">" to 4.0 / 18, ";" to 5.0 / 18, "!" to -3.0 / 18, " " to 0.25,
    "quad" to 1.0, "qquad" to 2.0, "thinspace" to 3.0 / 18, "medspace" to 4.0 / 18, "thickspace" to 5.0 / 18,
  )

  val delimiters: Map<String, String> = mapOf(
    "{" to "{", "}" to "}", "langle" to "⟨", "rangle" to "⟩", "lfloor" to "⌊", "rfloor" to "⌋", "lceil" to "⌈",
    "rceil" to "⌉", "|" to "∥", "vert" to "∣", "Vert" to "∥", "lvert" to "∣", "rvert" to "∣", "lbrace" to "{",
    "rbrace" to "}",
  )

  val bigOps: Map<String, Pair<String, Boolean>> = mapOf(
    "sum" to ("∑" to true), "prod" to ("∏" to true), "coprod" to ("∐" to true), "bigcup" to ("⋃" to true),
    "bigcap" to ("⋂" to true), "bigoplus" to ("⨁" to true), "bigotimes" to ("⨂" to true), "int" to ("∫" to false),
    "iint" to ("∬" to false), "oint" to ("∮" to false),
  )

  /** Operator names and whether they take limits in display style */
  val namedOps: Map<String, Boolean> = mapOf(
    "lim" to true, "max" to true, "min" to true, "sup" to true, "inf" to true, "det" to true, "gcd" to true,
    "Pr" to true, "sin" to false, "cos" to false, "tan" to false, "cot" to false, "sec" to false, "csc" to false,
    "sinh" to false, "cosh" to false, "tanh" to false, "log" to false, "ln" to false, "lg" to false, "exp" to false,
    "arg" to false, "deg" to false, "dim" to false, "ker" to false, "hom" to false, "arcsin" to false,
    "arccos" to false, "arctan" to false,
  )

  val accents: Map<String, String> = mapOf(
    "hat" to "ˆ", "widehat" to "ˆ", "bar" to "ˉ", "tilde" to "˜", "widetilde" to "˜", "dot" to "˙", "ddot" to "¨",
    "vec" to "⃗", "acute" to "ˊ", "grave" to "ˋ", "check" to "ˇ", "breve" to "˘",
  )

  val commands: Map<String, Command> = buildMap {
    val lowerGreek = listOf(
      "alpha" to "α", "beta" to "β", "gamma" to "γ", "delta" to "δ", "epsilon" to "ϵ", "varepsilon" to "ε",
      "zeta" to "ζ", "eta" to "η", "theta" to "θ", "vartheta" to "ϑ", "iota" to "ι", "kappa" to "κ", "lambda" to "λ",
      "mu" to "μ", "nu" to "ν", "xi" to "ξ", "pi" to "π", "varpi" to "ϖ", "rho" to "ρ", "varrho" to "ϱ",
      "sigma" to "σ", "varsigma" to "ς", "tau" to "τ", "upsilon" to "υ", "phi" to "ϕ", "varphi" to "φ", "chi" to "χ",
      "psi" to "ψ", "omega" to "ω",
    )
    for ((name, c) in lowerGreek) put(name, Command(c, KaTeXFace.Math, MathClass.Ord))
    val upperGreek = listOf(
      "Gamma" to "Γ", "Delta" to "Δ", "Theta" to "Θ", "Lambda" to "Λ", "Xi" to "Ξ", "Pi" to "Π", "Sigma" to "Σ",
      "Upsilon" to "Υ", "Phi" to "Φ", "Psi" to "Ψ", "Omega" to "Ω",
    )
    for ((name, c) in upperGreek) put(name, Command(c, KaTeXFace.Main, MathClass.Ord))
    val rel = listOf(
      "le" to "≤", "leq" to "≤", "ge" to "≥", "geq" to "≥", "ne" to "≠", "neq" to "≠", "approx" to "≈",
      "equiv" to "≡", "sim" to "∼", "simeq" to "≃", "cong" to "≅", "to" to "→", "rightarrow" to "→",
      "leftarrow" to "←", "gets" to "←", "Rightarrow" to "⇒", "Leftarrow" to "⇐", "leftrightarrow" to "↔",
      "Leftrightarrow" to "⇔", "iff" to "⟺", "implies" to "⟹", "mapsto" to "↦", "in" to "∈", "notin" to "∉",
      "ni" to "∋", "subset" to "⊂", "supset" to "⊃", "subseteq" to "⊆", "supseteq" to "⊇", "propto" to "∝",
      "ll" to "≪", "gg" to "≫", "perp" to "⊥", "mid" to "∣", "parallel" to "∥", "prec" to "≺", "succ" to "≻",
      "longrightarrow" to "⟶", "longleftarrow" to "⟵",
    )
    for ((name, c) in rel) put(name, Command(c, KaTeXFace.Main, MathClass.Rel))
    val bin = listOf(
      "cdot" to "⋅", "times" to "×", "pm" to "±", "mp" to "∓", "div" to "÷", "cup" to "∪", "cap" to "∩",
      "circ" to "∘", "ast" to "∗", "oplus" to "⊕", "otimes" to "⊗", "setminus" to "∖", "wedge" to "∧",
      "land" to "∧", "vee" to "∨", "lor" to "∨", "bullet" to "∙", "star" to "⋆",
    )
    for ((name, c) in bin) put(name, Command(c, KaTeXFace.Main, MathClass.Bin))
    val ord = listOf(
      "infty" to "∞", "partial" to "∂", "nabla" to "∇", "forall" to "∀", "exists" to "∃", "emptyset" to "∅",
      "varnothing" to "∅", "prime" to "′", "hbar" to "ℏ", "ell" to "ℓ", "angle" to "∠", "triangle" to "△",
      "neg" to "¬", "lnot" to "¬", "top" to "⊤", "bot" to "⊥", "aleph" to "ℵ", "Re" to "ℜ", "Im" to "ℑ",
      "wp" to "℘", "surd" to "√", "%" to "%", "$" to "$", "#" to "#", "&" to "&", "_" to "_",
    )
    for ((name, c) in ord) put(name, Command(c, KaTeXFace.Main, MathClass.Ord))
    for ((name, c) in listOf("ldots" to "…", "dots" to "…", "cdots" to "⋯", "vdots" to "⋮", "ddots" to "⋱")) {
      put(name, Command(c, KaTeXFace.Main, MathClass.Inner))
    }
    put("{", Command("{", KaTeXFace.Main, MathClass.Open))
    put("}", Command("}", KaTeXFace.Main, MathClass.Close))
    put("langle", Command("⟨", KaTeXFace.Main, MathClass.Open))
    put("rangle", Command("⟩", KaTeXFace.Main, MathClass.Close))
    put("lfloor", Command("⌊", KaTeXFace.Main, MathClass.Open))
    put("rfloor", Command("⌋", KaTeXFace.Main, MathClass.Close))
    put("lceil", Command("⌈", KaTeXFace.Main, MathClass.Open))
    put("rceil", Command("⌉", KaTeXFace.Main, MathClass.Close))
    put("|", Command("∥", KaTeXFace.Main, MathClass.Ord))
    put("vert", Command("∣", KaTeXFace.Main, MathClass.Ord))
    put("colon", Command(":", KaTeXFace.Main, MathClass.Punct))
  }
}

/** Something drawn: a glyph outline at a baseline point, or a filled rule (both y up from the box's baseline) */
internal sealed interface MathInk {
  fun offset(dx: Float, dy: Float): MathInk

  /** One code point in a typeface at `size` px, its origin on the baseline at (x, y) */
  class Glyph(val typeface: Typeface, val size: Float, val text: String, val x: Float, val y: Float) : MathInk {
    override fun offset(dx: Float, dy: Float) = Glyph(typeface, size, text, x + dx, y + dy)
  }

  /** A rectangle whose bottom edge is at `y` */
  class Rule(val x: Float, val y: Float, val width: Float, val height: Float) : MathInk {
    override fun offset(dx: Float, dy: Float) = Rule(x + dx, y + dy, width, height)
  }
}

/** A laid-out box; mutable like the Swift struct's `var`s, so take a [copy] where Swift copied the value */
internal class MathBox(
  var width: Float = 0f,
  var height: Float = 0f,
  var depth: Float = 0f,
  /** The italic correction already counted in `width` (KaTeX's margin-right on a slanted glyph) */
  var italic: Float = 0f,
  var cls: MathClass = MathClass.Ord,
  /** A single symbol: scripts sit at fixed shifts from it instead of hanging off its box */
  var isSymbol: Boolean = false,
  var ink: MutableList<MathInk> = mutableListOf(),
) {
  fun copy() = MathBox(width, height, depth, italic, cls, isSymbol, ink.toMutableList())

  /** Adds another box's ink with its baseline origin at (x, y) */
  fun place(box: MathBox, x: Float, y: Float) {
    for (piece in box.ink) ink += piece.offset(x, y)
  }

  /** Moves the glyphs (not the rules) down by `shift` */
  fun lowerGlyphs(shift: Float) {
    ink = ink.mapTo(mutableListOf()) { if (it is MathInk.Glyph) it.offset(0f, -shift) else it }
  }
}

/** KaTeX's styles: display, text, script, scriptscript, each possibly cramped */
internal data class MathStyle(val level: Int, val cramped: Boolean = false) {
  val isDisplay: Boolean get() = level == 0
  /** Script levels get tight spacing */
  val tight: Boolean get() = level >= 2
  val multiplier: Float get() = MULTIPLIERS[level]
  /** The column of the font parameter tables */
  val sizeIndex: Int get() = SIZE_INDEX[level]

  fun sup() = MathStyle(if (level < 2) 2 else 3, cramped)
  fun sub() = MathStyle(if (level < 2) 2 else 3, true)
  fun fracNum() = MathStyle(if (level == 0) 1 else if (level == 1) 2 else 3, cramped)
  fun fracDen() = MathStyle(if (level == 0) 1 else if (level == 1) 2 else 3, true)
  fun crampedStyle() = MathStyle(level, true)

  companion object {
    private val MULTIPLIERS = floatArrayOf(1f, 1f, 0.7f, 0.5f)
    private val SIZE_INDEX = intArrayOf(0, 0, 1, 2)
    val display = MathStyle(0)
  }
}

/** TeX inter-atom spacing classes (KaTeX spacingData); script styles keep only [TIGHT_THIN] */
private val THIN = setOf(
  MathClass.Ord to MathClass.Op, MathClass.Ord to MathClass.Inner, MathClass.Op to MathClass.Ord,
  MathClass.Op to MathClass.Op, MathClass.Op to MathClass.Inner, MathClass.Close to MathClass.Op,
  MathClass.Close to MathClass.Inner, MathClass.Inner to MathClass.Ord, MathClass.Inner to MathClass.Op,
  MathClass.Inner to MathClass.Open, MathClass.Inner to MathClass.Punct, MathClass.Inner to MathClass.Inner,
  MathClass.Punct to MathClass.Ord, MathClass.Punct to MathClass.Op, MathClass.Punct to MathClass.Open,
  MathClass.Punct to MathClass.Close, MathClass.Punct to MathClass.Punct, MathClass.Punct to MathClass.Inner,
)
private val MEDIUM = setOf(
  MathClass.Ord to MathClass.Bin, MathClass.Bin to MathClass.Ord, MathClass.Bin to MathClass.Op,
  MathClass.Bin to MathClass.Open, MathClass.Bin to MathClass.Inner, MathClass.Close to MathClass.Bin,
  MathClass.Inner to MathClass.Bin,
)
private val THICK = setOf(
  MathClass.Ord to MathClass.Rel, MathClass.Op to MathClass.Rel, MathClass.Rel to MathClass.Ord,
  MathClass.Rel to MathClass.Op, MathClass.Rel to MathClass.Open, MathClass.Rel to MathClass.Inner,
  MathClass.Close to MathClass.Rel, MathClass.Punct to MathClass.Rel, MathClass.Inner to MathClass.Rel,
)
private val TIGHT_THIN = setOf(
  MathClass.Ord to MathClass.Op, MathClass.Op to MathClass.Ord, MathClass.Op to MathClass.Op,
  MathClass.Close to MathClass.Op, MathClass.Inner to MathClass.Op,
)

/** The faces a symbol falls back through: its own, then Main, then AMS */
private val FALLBACK_FACES = listOf(KaTeXFace.Main, KaTeXFace.Ams)

/** `base` is the em of display and text style, in pixels */
internal class MathLayout(val base: Float, private val fonts: KaTeXFont) {
  /** Measures characters none of the KaTeX faces carry, in the system's fallback chain */
  private val fallbackPaint by lazy(LazyThreadSafetyMode.NONE) { Paint().apply { typeface = Typeface.DEFAULT } }

  fun size(s: MathStyle): Float = base * s.multiplier
  /** A font parameter in pixels for a style */
  fun param(name: String, s: MathStyle): Float = fonts.param(name, s.sizeIndex).toFloat() * size(s)

  fun layout(node: MathNode, s: MathStyle): MathBox = when (node) {
    is MathNode.Symbol -> symbol(node.c, node.face, s).also { it.cls = node.cls }
    is MathNode.Word -> hlist(node.text.codePointStrings().map { symbol(it, node.face, s, text = true) }).also { it.cls = node.cls }
    is MathNode.Row -> row(node.nodes, s)
    is MathNode.Scripts -> scripts(node.base, node.sup, node.sub, s)
    is MathNode.BigOp -> bigOp(node.c, s).first
    is MathNode.NamedOp ->
      hlist(node.name.codePointStrings().map { symbol(it, KaTeXFace.Main, s, text = true) }).also { it.cls = MathClass.Op }
    is MathNode.Frac -> frac(node.num, node.den, s)
    is MathNode.Sqrt -> sqrt(node.inner, s)
    is MathNode.Space -> MathBox(width = node.em.toFloat() * size(s))
    is MathNode.Delimited -> delimited(node.open, node.inner, node.close, s)
    is MathNode.Accent -> accent(node.mark, node.inner, s)
    is MathNode.Overline -> overline(node.inner, s)
  }

  /** A glyph with its TeX metrics; a character the face lacks falls back to Main, then to the system's fallback face */
  fun symbol(c: String, face: KaTeXFace, s: MathStyle, text: Boolean = false): MathBox {
    val pt = size(s)
    if (c.isEmpty()) return MathBox()
    if (c == " ") return MathBox(width = 0.25f * pt)
    val cp = c.codePointAt(0)
    for (f in listOf(face) + FALLBACK_FACES) {
      val m = fonts.metrics(f, cp) ?: continue
      // KaTeX keeps a slanted glyph's italic correction as a right margin, except inside \text
      val italic = if (text) 0f else m.italic.toFloat() * pt
      return MathBox(
        width = m.width.toFloat() * pt + italic, height = m.height.toFloat() * pt, depth = m.depth.toFloat() * pt,
        italic = italic, isSymbol = true, ink = mutableListOf(MathInk.Glyph(fonts.typeface(f), pt, c, 0f, 0f)),
      )
    }
    // Not in KaTeX's faces (CJK inside \text …): the system fallback, measured from the font
    val paint = fallbackPaint.apply { textSize = pt }
    if (!paint.hasGlyph(c)) return MathBox()
    val fm = paint.fontMetrics
    return MathBox(
      width = paint.measureText(c), height = -fm.ascent, depth = fm.descent, isSymbol = true,
      ink = mutableListOf(MathInk.Glyph(Typeface.DEFAULT, pt, c, 0f, 0f)),
    )
  }

  /** Boxes side by side on one baseline, no spacing */
  fun hlist(boxes: List<MathBox>): MathBox {
    val out = MathBox()
    for (b in boxes) {
      out.place(b, out.width, 0f)
      out.width += b.width
      out.height = max(out.height, b.height)
      out.depth = max(out.depth, b.depth)
    }
    if (boxes.size == 1) {
      out.italic = boxes[0].italic
      out.isSymbol = boxes[0].isSymbol
    }
    return out
  }

  /** A math list with TeX's inter-atom spacing (KaTeX spacingData) */
  fun row(nodes: List<MathNode>, s: MathStyle): MathBox {
    val boxes = nodes.map { layout(it, s) }
    val isSpace = nodes.map { it is MathNode.Space }
    // A binary operator with nothing to bind on one side is an ordinary symbol
    var previous: MathClass? = null
    for (i in boxes.indices) {
      if (isSpace[i]) continue
      if (boxes[i].cls == MathClass.Bin) {
        val next = (i + 1 until boxes.size).firstOrNull { !isSpace[it] }?.let { boxes[it].cls }
        if (previous == null || previous in BIN_AFTER || next == null || next in BIN_BEFORE) boxes[i].cls = MathClass.Ord
      }
      previous = boxes[i].cls
    }
    val out = MathBox()
    var last: MathClass? = null
    for (i in boxes.indices) {
      val box = boxes[i]
      if (!isSpace[i] && last != null) out.width += spacing(last, box.cls, s)
      out.place(box, out.width, 0f)
      out.width += box.width
      out.height = max(out.height, box.height)
      out.depth = max(out.depth, box.depth)
      if (!isSpace[i]) last = box.cls
    }
    if (boxes.size == 1) {
      out.italic = boxes[0].italic
      out.isSymbol = boxes[0].isSymbol
      out.cls = boxes[0].cls
    }
    return out
  }

  /** thin 3mu, medium 4mu, thick 5mu between two atom classes; script styles keep only the thin ones around operators */
  fun spacing(a: MathClass, b: MathClass, s: MathStyle): Float {
    val mu = param("quad", s) / 18
    val pair = a to b
    if (s.tight) return if (pair in TIGHT_THIN) 3 * mu else 0f
    return when (pair) {
      in THIN -> 3 * mu
      in MEDIUM -> 4 * mu
      in THICK -> 5 * mu
      else -> 0f
    }
  }

  /** A big operator's glyph, centred on the axis (KaTeX op.js); returns the shift it was moved down by */
  fun bigOp(c: String, s: MathStyle): Pair<MathBox, Float> {
    val box = symbol(c, if (s.isDisplay) KaTeXFace.Size2 else KaTeXFace.Size1, s)
    val shift = (box.height - box.depth) / 2 - param("axisHeight", s)
    box.height -= shift
    box.depth += shift
    box.lowerGlyphs(shift)
    box.cls = MathClass.Op
    box.isSymbol = false
    return box to shift
  }

  fun scripts(baseNode: MathNode?, supNode: MathNode?, subNode: MathNode?, s: MathStyle): MathBox {
    // Limits above and below: a display-style operator that takes them
    if (baseNode != null && s.isDisplay) {
      if (baseNode is MathNode.BigOp && baseNode.limits) {
        val (op, shift) = bigOp(baseNode.c, s)
        return limits(op, shift, op.italic, supNode, subNode, s)
      }
      if (baseNode is MathNode.NamedOp && baseNode.limits) {
        return limits(layout(baseNode, s), 0f, 0f, supNode, subNode, s)
      }
    }
    val base = baseNode?.let { layout(it, s) } ?: MathBox()
    val supStyle = s.sup()
    val subStyle = s.sub()
    val sup = supNode?.let { layout(it, supStyle) }
    val sub = subNode?.let { layout(it, subStyle) }
    // KaTeX supsub.js
    var supShift = 0f
    var subShift = 0f
    if (!base.isSymbol) {
      supShift = base.height - param("supDrop", supStyle)
      subShift = base.depth + param("subDrop", subStyle)
    }
    val xHeight = param("xHeight", s)
    val minSup = if (s.isDisplay) param("sup1", s) else if (s.cramped) param("sup3", s) else param("sup2", s)
    if (sup != null && sub != null) {
      supShift = maxOf(supShift, minSup, sup.depth + 0.25f * xHeight)
      subShift = max(subShift, param("sub2", s))
      val rule = param("defaultRuleThickness", s)
      if ((supShift - sup.depth) - (sub.height - subShift) < 4 * rule) {
        subShift = 4 * rule - (supShift - sup.depth) + sub.height
        val psi = 0.8f * xHeight - (supShift - sup.depth)
        if (psi > 0) {
          supShift += psi
          subShift -= psi
        }
      }
    } else if (sub != null) {
      subShift = maxOf(subShift, param("sub1", s), sub.height - 0.8f * xHeight)
    } else if (sup != null) {
      supShift = maxOf(supShift, minSup, sup.depth + 0.25f * xHeight)
    }
    // \scriptspace after the scripts
    val scriptSpace = SCRIPT_SPACE * this.base
    val x = base.width
    val out = base.copy()
    out.isSymbol = false
    out.italic = 0f
    var right = x
    if (sup != null) {
      out.place(sup, x, supShift)
      right = max(right, x + sup.width)
      out.height = max(out.height, supShift + sup.height)
      out.depth = max(out.depth, sup.depth - supShift)
    }
    if (sub != null) {
      // A slanted symbol's subscript tucks under it
      val subX = x - if (base.isSymbol) base.italic else 0f
      out.place(sub, subX, -subShift)
      right = max(right, subX + sub.width)
      out.depth = max(out.depth, subShift + sub.depth)
      out.height = max(out.height, sub.height - subShift)
    }
    out.width = right + scriptSpace
    return out
  }

  /** Scripts centred above and below an operator (KaTeX assembleSupSub) */
  @Suppress("UNUSED_PARAMETER")
  fun limits(op: MathBox, shift: Float, slant: Float, supNode: MathNode?, subNode: MathNode?, s: MathStyle): MathBox {
    val sup = supNode?.let { layout(it, s.sup()) }
    val sub = subNode?.let { layout(it, s.sub()) }
    val width = maxOf(op.width, sup?.width ?: 0f, sub?.width ?: 0f)
    val out = MathBox(width = width, height = op.height, depth = op.depth, cls = MathClass.Op)
    out.place(op, (width - op.width) / 2, 0f)
    val spacing5 = param("bigOpSpacing5", s)
    if (sup != null) {
      val kern = max(param("bigOpSpacing1", s), param("bigOpSpacing3", s) - sup.depth)
      val y = op.height + kern + sup.depth
      out.place(sup, (width - sup.width) / 2 + slant / 2, y)
      out.height = y + sup.height + spacing5
    }
    if (sub != null) {
      val kern = max(param("bigOpSpacing2", s), param("bigOpSpacing4", s) - sub.height)
      val y = -(op.depth + kern + sub.height)
      out.place(sub, (width - sub.width) / 2 - slant / 2, y)
      out.depth = -y + sub.depth + spacing5
    }
    return out
  }

  /** KaTeX genfrac.js with a bar */
  fun frac(numNode: MathNode, denNode: MathNode, s: MathStyle): MathBox {
    val num = layout(numNode, s.fracNum())
    val den = layout(denNode, s.fracDen())
    val rule = param("defaultRuleThickness", s)
    val axis = param("axisHeight", s)
    var numShift = if (s.isDisplay) param("num1", s) else param("num2", s)
    var denShift = if (s.isDisplay) param("denom1", s) else param("denom2", s)
    val clearance = if (s.isDisplay) 3 * rule else rule
    val numGap = (numShift - num.depth) - (axis + rule / 2)
    if (numGap < clearance) numShift += clearance - numGap
    val denGap = (axis - rule / 2) - (den.height - denShift)
    if (denGap < clearance) denShift += clearance - denGap
    // \nulldelimiterspace on each side
    val side = NULL_DELIMITER_SPACE * size(s)
    val inner = max(num.width, den.width)
    val out = MathBox(width = inner + 2 * side, height = numShift + num.height, depth = denShift + den.depth)
    out.place(num, side + (inner - num.width) / 2, numShift)
    out.place(den, side + (inner - den.width) / 2, -denShift)
    out.ink += MathInk.Rule(side, axis - rule / 2, inner, rule)
    return out
  }

  /**
   * A radical: the smallest surd glyph that covers the body, its rule running over the body (KaTeX sqrt.js, glyphs
   * in place of its SVG paths)
   */
  fun sqrt(innerNode: MathNode, s: MathStyle): MathBox {
    val inner = layout(innerNode, s.crampedStyle())
    val theta = param("defaultRuleThickness", s)
    val phi = if (s.isDisplay) param("xHeight", s) else theta
    var clearance = theta + phi / 4
    val needed = inner.height + inner.depth + clearance + theta
    var surd = symbol("√", KaTeXFace.Main, s)
    for (face in SURD_FACES) {
      surd = symbol("√", face, s)
      if (surd.height + surd.depth >= needed) break
    }
    val total = surd.height + surd.depth
    if (total - theta > inner.height + inner.depth + clearance) {
      clearance = (clearance + total - theta - inner.height - inner.depth) / 2
    }
    // The surd's top meets the rule's top
    val top = inner.height + clearance + theta
    val out = MathBox(width = surd.width + inner.width, height = top, depth = max(inner.depth, total - top))
    out.place(surd, 0f, top - surd.height)
    out.place(inner, surd.width, 0f)
    out.ink += MathInk.Rule(surd.width, top - theta, inner.width, theta)
    return out
  }

  /** \left … \right: delimiters grown to the body around the axis (KaTeX delimiter.js, Main / Size1 / Size2) */
  fun delimited(open: String?, innerNode: MathNode, close: String?, s: MathStyle): MathBox {
    val inner = layout(innerNode, s)
    val axis = param("axisHeight", s)
    val reach = max(inner.height - axis, inner.depth + axis)
    val wanted = max(reach / DELIMITER_UNIT * DELIMITER_FACTOR * 2, 2 * reach - DELIMITER_SHORTFALL * size(s))
    fun delim(c: String?): MathBox {
      if (c == null) return MathBox(width = NULL_DELIMITER_SPACE * size(s))
      var box = symbol(c, KaTeXFace.Main, s)
      for (face in SURD_FACES) {
        if (c.isEmpty() || fonts.metrics(face, c.codePointAt(0)) == null) continue
        box = symbol(c, face, s)
        if (box.height + box.depth >= wanted) break
      }
      // Centred on the axis
      val shift = (box.height - box.depth) / 2 - axis
      box.height -= shift
      box.depth += shift
      box.lowerGlyphs(shift)
      return box
    }
    val out = hlist(listOf(delim(open), inner, delim(close)))
    out.cls = MathClass.Inner
    out.isSymbol = false
    return out
  }

  /** An accent glyph centred over its base, shifted by the base's skew (KaTeX accent.js) */
  fun accent(mark: String, innerNode: MathNode, s: MathStyle): MathBox {
    val inner = layout(innerNode, s.crampedStyle())
    val accent = symbol(mark, KaTeXFace.Main, s, text = true)
    var skew = 0f
    val single = symbolNode(innerNode)
    if (inner.isSymbol && single != null && single.c.isNotEmpty()) {
      fonts.metrics(single.face, single.c.codePointAt(0))?.let { skew = it.skew.toFloat() * size(s) }
    }
    val clearance = min(inner.height, param("xHeight", s))
    val out = inner.copy()
    out.isSymbol = false
    val y = inner.height - clearance
    out.place(accent, (inner.width - inner.italic - accent.width) / 2 + skew, y)
    out.height = max(inner.height, y + accent.height)
    return out
  }

  private fun symbolNode(node: MathNode): MathNode.Symbol? = when {
    node is MathNode.Row && node.nodes.size == 1 -> symbolNode(node.nodes[0])
    node is MathNode.Symbol -> node
    else -> null
  }

  fun overline(innerNode: MathNode, s: MathStyle): MathBox {
    val inner = layout(innerNode, s.crampedStyle())
    val rule = param("defaultRuleThickness", s)
    val out = inner.copy()
    out.isSymbol = false
    out.ink += MathInk.Rule(0f, inner.height + 3 * rule, inner.width, rule)
    out.height = inner.height + 5 * rule
    return out
  }

  companion object {
    /** A binary operator after one of these, or before one of [BIN_BEFORE], is ordinary */
    private val BIN_AFTER = setOf(MathClass.Bin, MathClass.Op, MathClass.Rel, MathClass.Open, MathClass.Punct)
    private val BIN_BEFORE = setOf(MathClass.Rel, MathClass.Close, MathClass.Punct)
    /** The faces a surd or a \left / \right delimiter grows through */
    private val SURD_FACES = listOf(KaTeXFace.Main, KaTeXFace.Size1, KaTeXFace.Size2)
    /** \scriptspace, in em of display style */
    private const val SCRIPT_SPACE = 0.05f
    /** \nulldelimiterspace, in em */
    private const val NULL_DELIMITER_SPACE = 0.12f
    /** \delimiterfactor (901 / 1000 of the reach, here in KaTeX's /500 × 2 form) and \delimitershortfall (0.5 em) */
    private const val DELIMITER_UNIT = 500f
    private const val DELIMITER_FACTOR = 901f
    private const val DELIMITER_SHORTFALL = 0.5f
  }
}

/** A laid-out formula as fill paths in canvas space (y down, the baseline `ascent` below the top) */
internal class MathDrawing(val width: Float, val ascent: Float, val descent: Float, val glyphs: Path, val rules: Path) {
  val height: Float get() = ascent + descent
}

/**
 * Flattens a box into outline paths once, so a frame only fills them. Glyph outlines come from Paint.getTextPath with
 * hinting off (the unhinted outline CTFontCreatePathForGlyph gives on iOS); rules go in their own path so a rule
 * overlapping a glyph cannot cancel it out under the nonzero fill.
 */
internal fun MathBox.drawing(): MathDrawing {
  val glyphs = android.graphics.Path()
  val rules = android.graphics.Path()
  val outline = android.graphics.Path()
  val paint = Paint().apply { hinting = Paint.HINTING_OFF }
  for (piece in ink) {
    when (piece) {
      is MathInk.Glyph -> {
        paint.typeface = piece.typeface
        paint.textSize = piece.size
        // Flip from TeX's y-up baseline space into the canvas, the baseline `height` below the top
        paint.getTextPath(piece.text, 0, piece.text.length, piece.x, height - piece.y, outline)
        glyphs.addPath(outline)
      }
      is MathInk.Rule ->
        rules.addRect(piece.x, height - (piece.y + piece.height), piece.x + piece.width, height - piece.y, android.graphics.Path.Direction.CW)
    }
  }
  return MathDrawing(width, height, depth, glyphs.asComposePath(), rules.asComposePath())
}

/**
 * `$$` display math: KaTeX's display block at the reply's size × [MathTokens.displayScale], centred, scrolling
 * sideways when wider than the column (the column itself never widens); `.katex-display` keeps a 2 px pad above and
 * below
 */
@Composable
fun MathBlock(tex: String, modifier: Modifier = Modifier) {
  val assets = LocalContext.current.assets
  val density = LocalDensity.current
  val base = with(density) { WebType.text1.fontSize.toPx() } * MathTokens.displayScale
  val drawing = remember(tex, base) {
    MathLayout(base, KaTeXFont.get(assets)).layout(TeXParser.parse(tex), MathStyle.display).drawing()
  }
  val ink = Palette.fg1
  val width = ceil(drawing.width)
  val canvas = with(density) { Modifier.size(width.toDp(), ceil(drawing.height).toDp()) }
    .clearAndSetSemantics {}
    .drawBehind {
      drawPath(drawing.glyphs, ink)
      drawPath(drawing.rules, ink)
    }
  BoxWithConstraints(modifier.fillMaxWidth().padding(vertical = Metrics.s0_5), contentAlignment = Alignment.Center) {
    if (!constraints.hasBoundedWidth || width <= constraints.maxWidth) {
      Spacer(canvas)
    } else {
      Box(Modifier.fillMaxWidth().horizontalScroll(rememberScrollState())) { Spacer(canvas) }
    }
  }
}
