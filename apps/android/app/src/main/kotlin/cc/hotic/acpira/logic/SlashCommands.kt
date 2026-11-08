package cc.hotic.acpira.logic

import cc.hotic.acpira.model.FileHit
import cc.hotic.acpira.model.SettingsView
import cc.hotic.acpira.model.SlashCommand
import cc.hotic.acpira.model.AgentInfo
import cc.hotic.acpira.model.SessionOption

// Ported from apps/ios Logic/SlashCommands.swift (slashCommands.ts, Mention.tsx and personaHits.ts): the `/` command and
// `@` mention tokens under the caret, the command list's ranking, completion and input hint, the marks the field
// paints as pills, and the summonable personas offered ahead of the files. Offsets count UTF-16 units, as the field's
// selection does.

/** A `/` or `@` token under the caret: where its sigil sits and what has been typed after it, up to the caret */
data class SigilSpan(val start: Int, val query: String)

/** The prompt after a pick, and where the caret goes */
data class PromptEdit(val text: String, val caret: Int)

/** A `/command` or `@persona` token the composer paints as a pill; `start` is the sigil's offset */
data class PromptMark(val start: Int, val name: String, val summon: Boolean = false) {
  /** The sigil and the name */
  val length: Int get() = name.length + 1
}

object SlashCommands {
  private val scope = Regex("^(?:agents|claude):")
  private val lastCommand = Regex("(?:^|\\s)/(\\S+)\\s*$")

  /**
   * What may sit right before a sigil for it to count (SIGIL_LEAD): whitespace, or CJK text and its full-width
   * punctuation, which those languages write without spaces (`问问@GPT`, `然后/review`). Latin letters, digits and ASCII
   * punctuation keep `a@b.com`, `a/b` and `https://` plain
   */
  private val cjk = listOf(
    0x1100..0x11ff, 0x2e80..0x2fdf, 0x3000..0x30ff, 0x3130..0x318f, 0x31f0..0x31ff, 0x3400..0x4dbf, 0x4e00..0x9fff,
    0xac00..0xd7af, 0xf900..0xfaff, 0xff00..0xffef, 0x20000..0x323af,
  )

  /** Whether what stands before offset [i] of [text] lets a sigil there count; the text's start always does */
  fun leadsAt(text: CharSequence, i: Int): Boolean {
    if (i <= 0) return true
    val c = text[i - 1]
    if (c.isWhitespace()) return true
    val cp = Character.codePointBefore(text, i)
    return cjk.any { cp in it }
  }

  /** Where the trailing run of the text before the caret that holds no whitespace starts */
  private fun runStart(head: String): Int {
    var from = head.length
    while (from > 0 && !head[from - 1].isWhitespace()) from--
    return from
  }

  /** commandAt: the first `/` in the run before the caret that leads; the query runs from it to the caret */
  fun commandAt(text: String, caret: Int): SigilSpan? {
    val head = text.take(caret.coerceIn(0, text.length))
    for (i in runStart(head) until head.length) {
      if (head[i] == '/' && leadsAt(head, i)) return SigilSpan(i, head.substring(i + 1))
    }
    return null
  }

  /** mentionAt: the last `@` in the run before the caret, when it leads; the query holds no further `@` */
  fun mentionAt(text: String, caret: Int): SigilSpan? {
    val head = text.take(caret.coerceIn(0, text.length))
    val from = runStart(head)
    val i = head.lastIndexOf('@')
    if (i < from || !leadsAt(head, i)) return null
    return SigilSpan(i, head.substring(i + 1))
  }

  /** completeCommand: the token becomes `/name ` (the rest of it after the caret goes too); what follows stays */
  fun complete(text: String, span: SigilSpan, caret: Int, name: String): PromptEdit {
    val head = text.take(span.start.coerceAtMost(text.length)) + "/$name "
    val rest = text.substring(caret.coerceIn(0, text.length)).dropWhile { !it.isWhitespace() }.dropWhile { it.isWhitespace() }
    return PromptEdit(head + rest, head.length)
  }

  /**
   * matchCommands: name prefixes first, then names containing the query or descriptions with a word starting with it,
   * each tier in the agent's order; case-insensitive. An empty query lists everything
   */
  fun match(commands: List<SlashCommand>, query: String): List<SlashCommand> {
    val q = query.lowercase()
    if (q.isEmpty()) return mergeSkillCopies(commands)
    fun wordStart(s: String) = s.lowercase().split(Regex("[^\\p{L}\\p{N}]+")).any { it.isNotEmpty() && it.startsWith(q) }
    val prefix = commands.filter { it.name.lowercase().startsWith(q) }
    val rest = commands.filter { c ->
      val n = c.name.lowercase()
      !n.startsWith(q) && (n.contains(q) || wordStart(c.description))
    }
    return mergeSkillCopies(prefix + rest)
  }

  /** A skill listed from both `~/.agents/skills` and `~/.claude/skills` (`agents:dig`, `claude:dig`) shows once */
  private fun mergeSkillCopies(commands: List<SlashCommand>): List<SlashCommand> {
    val seen = HashSet<String>()
    return commands.filter { c ->
      val m = scope.find(c.name) ?: return@filter true
      seen.add(c.name.substring(m.range.last + 1))
    }
  }

  /** commandHint: the input hint of the command the prompt's last token names, while its arguments are still empty */
  fun hint(commands: List<SlashCommand>, text: String): String? {
    val name = lastCommand.find(text)?.groupValues?.get(1) ?: return null
    return commands.firstOrNull { it.name == name }?.input?.hint?.takeIf { it.isNotEmpty() }
  }

  /** A summon (`@name `) written at [at], replacing [cut] characters there; a space before it unless one is there */
  fun summon(text: String, name: String, at: Int, cut: Int = 0): PromptEdit {
    val before = text.take(at.coerceIn(0, text.length))
    val lead = if (before.isEmpty() || before.last().isWhitespace()) "" else " "
    val token = "$lead@$name "
    val after = text.substring((at + cut).coerceIn(0, text.length))
    return PromptEdit(before + token + after, before.length + token.length)
  }

  /** A file picked for the `@` token: the token leaves the text (the file rides along as an attachment) */
  fun dropToken(text: String, span: SigilSpan, caret: Int): PromptEdit {
    val start = span.start.coerceAtMost(text.length)
    return PromptEdit(text.take(start) + text.substring(caret.coerceIn(start, text.length)), start)
  }

  // Marks

  private fun nameChar(c: Char) = c.isLetterOrDigit() || c in "_.:-"

  /**
   * commandMarks: every `/name` token naming an advertised command. The `/` leads, the name is an optional `$` then a
   * letter or digit and name characters, ending at whitespace or the end, so `a/b`, `/tmp/x` and `/review.` stay plain
   */
  fun commandMarks(names: List<String>, text: String): List<PromptMark> {
    if (names.isEmpty() || '/' !in text) return emptyList()
    val known = names.toSet()
    val marks = ArrayList<PromptMark>()
    var i = 0
    while (i < text.length) {
      if (text[i] == '/' && leadsAt(text, i)) {
        var j = i + 1
        if (j < text.length && text[j] == '$') j++
        if (j < text.length && text[j].isLetterOrDigit()) {
          while (j < text.length && nameChar(text[j])) j++
          if (j == text.length || text[j].isWhitespace()) {
            val name = text.substring(i + 1, j)
            if (name in known) marks += PromptMark(i, name)
            i = j
            continue
          }
        }
      }
      i++
    }
    return marks
  }

  /** summonMarks: every `@name` of a summonable persona, longest names first; a match must not run on into a name */
  fun summonMarks(names: List<String>, text: String): List<PromptMark> {
    if (names.isEmpty() || '@' !in text) return emptyList()
    val sorted = names.filter { it.isNotEmpty() }.distinct().sortedByDescending { it.length }
    fun tail(c: Char) = c.isLetterOrDigit() || c in "_.:/@-"
    val marks = ArrayList<PromptMark>()
    var i = 0
    while (i < text.length) {
      if (text[i] == '@' && leadsAt(text, i)) {
        val hit = sorted.firstOrNull { name ->
          val end = i + 1 + name.length
          end <= text.length && text.regionMatches(i + 1, name, 0, name.length) && (end == text.length || !tail(text[end]))
        }
        if (hit != null) {
          marks += PromptMark(i, hit, summon = true)
          i += hit.length + 1
          continue
        }
      }
      i++
    }
    return marks
  }

  /** promptMarks: commands and summons in text order; a summon owns its whole span */
  fun promptMarks(commands: List<String>, summons: List<String>, text: String): List<PromptMark> {
    val summoned = summonMarks(summons, text)
    val commanded = commandMarks(commands, text)
    if (summoned.isEmpty()) return commanded
    val free = commanded.filter { c -> summoned.none { c.start > it.start && c.start <= it.start + it.name.length } }
    return (summoned + free).sortedBy { it.start }
  }

  /**
   * markRoom: how far mark [i]'s pill may reach past its text on each side. Each side allows its whitespace run
   * ([space] per character) less [clear], halved when another mark shares the run; a text edge or a line break allows
   * the full [overhang]. Both sides take the smaller, so a run of marks reads as even pills with even gaps
   */
  fun markReach(text: String, marks: List<PromptMark>, i: Int, overhang: Float, space: Float, clear: Float): Float {
    val m = marks[i]
    val before = if (i > 0) marks[i - 1].start + marks[i - 1].length else 0
    val after = if (i + 1 < marks.size) marks[i + 1].start else text.length
    fun blank(c: Char) = c.isWhitespace() && c != '\n' && c != '\r'
    fun side(from: Int, to: Int, leading: Boolean, markBeyond: Boolean): Float? {
      val gap = text.substring(minOf(from, to), to)
      val run = if (leading) gap.reversed().takeWhile(::blank).length else gap.takeWhile(::blank).length
      val edge = gap.length == run && !markBeyond
      val past = if (leading) gap.length - run - 1 else run
      val broken = past in gap.indices && (gap[past] == '\n' || gap[past] == '\r')
      if (edge || broken) return null
      val room = run * space - clear
      return if (markBeyond && gap.length == run) room / 2 else room
    }
    val sides = listOfNotNull(
      side(before, m.start, leading = true, markBeyond = i > 0),
      side(m.start + m.length, after, leading = false, markBeyond = i + 1 < marks.size),
    )
    return maxOf(0f, (sides + overhang).min())
  }
}

/**
 * A cross-harness subagent (or a group of them) as the `@` list and the summon bean offer it (personaHits.ts
 * MentionPersona): picking one writes `@name `, which the agent reads as "summon this one"
 */
data class MentionPersona(
  val id: String,
  val name: String,
  /** The CLI it runs in (empty for a group) */
  val agent: String,
  /** The CLI's name, or a group's members joined */
  val cli: String? = null,
  val model: String? = null,
  val effort: String? = null,
  val group: Boolean = false,
) {
  companion object {
    /** A persona's hit travels as a FileHit with this scheme */
    const val SCHEME = "acpira-agent:"

    /** App.tsx personas: the enabled personas, then the enabled groups with an enabled member */
    fun from(settings: SettingsView, agents: List<AgentInfo>, listSep: String): List<MentionPersona> {
      val on = settings.subagents.filter { it.enabled }
      val groups = settings.subagentGroups.mapNotNull { g ->
        val names = g.members.mapNotNull { m -> on.firstOrNull { it.id == m }?.name }
        if (!g.enabled || names.isEmpty()) null else MentionPersona(g.id, g.name, "", names.joinToString(listSep), group = true)
      }
      return on.map { p ->
        MentionPersona(
          p.id, p.name, p.agent, agents.firstOrNull { it.id == p.agent }?.name ?: p.agent, p.model,
          p.effort?.let { ComposerControls.effortLabel(SessionOption(id = it, name = it)) },
        )
      } + groups
    }

    /** personaHits: the personas whose name or id holds the query, leading the file list (a bare `@` lists them all) */
    fun hits(personas: List<MentionPersona>, query: String): List<FileHit> {
      val q = query.lowercase()
      return personas.filter { q.isEmpty() || it.name.lowercase().contains(q) || it.id.contains(q) }.map { FileHit(SCHEME + it.id, it.name) }
    }

    /** personaOfHit */
    fun of(hit: FileHit, personas: List<MentionPersona>): MentionPersona? {
      if (!hit.uri.startsWith(SCHEME)) return null
      val id = hit.uri.removePrefix(SCHEME)
      return personas.firstOrNull { it.id == id }
    }
  }
}
