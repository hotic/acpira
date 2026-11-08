package cc.hotic.acpira.logic

import cc.hotic.acpira.model.SlashCommand
import org.junit.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull

/** The composer's `/` and `@` logic and long pastes, case for case with apps/ios ComposerTests / PastedTextTests */
class SlashCommandsTest {
  private val commands = listOf(
    SlashCommand("review", "Review the changes", SlashCommand.Input("<branch>")),
    SlashCommand("compact", "Summarize the conversation"),
    SlashCommand("agents:dig", "Dig into a bug"),
    SlashCommand("claude:dig", "Dig into a bug"),
    SlashCommand("init", "Create a review file"),
  )

  @Test fun commandTokenUnderTheCaret() {
    assertEquals(SigilSpan(0, "rev"), SlashCommands.commandAt("/rev", 4))
    assertEquals(SigilSpan(3, "re"), SlashCommands.commandAt("hi /re", 6))
    assertEquals(SigilSpan(0, "tmp/x"), SlashCommands.commandAt("/tmp/x", 6))
    assertNull(SlashCommands.commandAt("a/b", 3))
    assertNull(SlashCommands.commandAt("https://x", 9))
    assertEquals(SigilSpan(2, "review"), SlashCommands.commandAt("然后/review", 9))
    assertNull(SlashCommands.commandAt("/review now", 11))
  }

  @Test fun mentionTokenUnderTheCaret() {
    assertEquals(SigilSpan(4, "gp"), SlashCommands.mentionAt("ask @gp", 7))
    assertEquals(SigilSpan(0, ""), SlashCommands.mentionAt("@", 1))
    assertNull(SlashCommands.mentionAt("a@b.com", 7))
    assertEquals(SigilSpan(2, "GPT"), SlashCommands.mentionAt("问问@GPT", 6))
    assertEquals(SigilSpan(0, "foo"), SlashCommands.mentionAt("@foo bar", 4))
    assertNull(SlashCommands.mentionAt("@a@b", 4))
  }

  @Test fun completingACommandKeepsItsArguments() {
    assertEquals(PromptEdit("/review ", 8), SlashCommands.complete("/re", SigilSpan(0, "re"), 3, "review"))
    assertEquals(PromptEdit("/review view x", 8), SlashCommands.complete("/re view x", SigilSpan(0, "re"), 3, "review"))
    assertEquals(PromptEdit("/review main", 8), SlashCommands.complete("/reXX main", SigilSpan(0, "re"), 3, "review"))
  }

  @Test fun matchingRanksPrefixesFirstAndMergesSkillCopies() {
    assertEquals(listOf("review", "init"), SlashCommands.match(commands, "re").map { it.name })
    assertEquals(listOf("review", "init"), SlashCommands.match(commands, "RE").map { it.name })
    assertEquals(listOf("agents:dig"), SlashCommands.match(commands, "dig").map { it.name })
    assertEquals(listOf("review", "compact", "agents:dig", "init"), SlashCommands.match(commands, "").map { it.name })
    assertEquals(emptyList(), SlashCommands.match(commands, "zzz"))
  }

  @Test fun inputHintWhileTheArgumentsAreEmpty() {
    assertEquals("<branch>", SlashCommands.hint(commands, "/review "))
    assertEquals("<branch>", SlashCommands.hint(commands, "please /review"))
    assertNull(SlashCommands.hint(commands, "/review main"))
    assertNull(SlashCommands.hint(commands, "/compact "))
  }

  @Test fun summonsAndDroppedFileTokens() {
    assertEquals(PromptEdit("ask @GPT ", 9), SlashCommands.summon("ask", "GPT", 3))
    assertEquals(PromptEdit("@GPT ", 5), SlashCommands.summon("", "GPT", 0))
    assertEquals(PromptEdit("hi @GPT ", 8), SlashCommands.summon("hi @gp", "GPT", 3, cut = 3))
    assertEquals(PromptEdit("see ", 4), SlashCommands.dropToken("see @Comp", SigilSpan(4, "Comp"), 9))
  }

  @Test fun marksWhereverTheyStand() {
    val names = listOf("review", "compact", "skill:git-commit", "\$git-commit")
    assertEquals(listOf(PromptMark(0, "review")), SlashCommands.commandMarks(names, "/review files"))
    assertEquals(listOf(PromptMark(0, "compact"), PromptMark(12, "review")), SlashCommands.commandMarks(names, "/compact 然后 /review"))
    assertEquals(listOf(PromptMark(3, "skill:git-commit")), SlashCommands.commandMarks(names, "拆分 /skill:git-commit 提交"))
    for (plain in listOf("foo /rev bar", "a/review", "look at /tmp/review.ts", "foo /review.", "a /$ b /$\$x")) {
      assertEquals(emptyList(), SlashCommands.commandMarks(names, plain), plain)
    }
    assertEquals(listOf(PromptMark(2, "审查", summon = true)), SlashCommands.summonMarks(listOf("审查"), "叫 @审查，看看"))
    assertEquals(emptyList(), SlashCommands.summonMarks(listOf("code"), "@code-review x"))
    assertEquals(listOf(PromptMark(0, "code review", summon = true)), SlashCommands.summonMarks(listOf("code", "code review"), "@code review now"))
    assertEquals(
      listOf(PromptMark(0, "review"), PromptMark(11, "审查", summon = true)),
      SlashCommands.promptMarks(listOf("review"), listOf("审查"), "/review 然后 @审查 看"),
    )
  }

  @Test fun longPastesBecomeAttachments() {
    for (text in listOf("", "你好", "/compact", "a".repeat(1999), List(19) { "line" }.joinToString("\n"))) {
      assertEquals(PastedText.Result.Inline, PastedText.collect(text))
    }
    for (text in listOf("中".repeat(2000)) + listOf("\n", "\r\n", "\r").map { sep -> List(20) { "  内容  " }.joinToString(sep) }) {
      assertEquals(PastedText.Result.Attachment, PastedText.collect(text))
    }
    assertEquals(PastedText.Result.Attachment, PastedText.collect("a".repeat(PastedText.MAX_BYTES)))
    assertEquals(PastedText.Result.TooBig, PastedText.collect("a".repeat(PastedText.MAX_BYTES + 1)))
  }
}
