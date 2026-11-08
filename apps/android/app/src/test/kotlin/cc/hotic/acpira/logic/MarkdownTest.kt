package cc.hotic.acpira.logic

import org.junit.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue
import kotlin.test.fail

/** The markdown subset the transcript renders, including half-streamed input. Mirrors apps/ios MarkdownTests.swift */
class MarkdownTest {
  private fun t(s: String) = MdInline.Text(s)
  private fun p(vararg i: MdInline) = MdBlock.Paragraph(i.toList())

  @Test fun blocks() {
    val doc = Markdown.parse(
      """
      ## 改动总结

      **核心变更**：`parseArgs` 新增 `dryRun` 字段

      - `--dry-run` 只置位
      - 详见 [文档](https://example.com/docs)

      > 注意：互斥

      | 参数 | 旧行为 |
      | --- | --- |
      | `--dry-run` | 报错 |

      1. 删除了
      2. README

      ```ts
      const x = 1
      ```
      """.trimIndent(),
    )
    assertEquals(7, doc.size)
    assertEquals(MdBlock.Heading(2, listOf(t("改动总结"))), doc[0])
    assertEquals(
      p(MdInline.Strong(listOf(t("核心变更"))), t("："), MdInline.Code("parseArgs"), t(" 新增 "), MdInline.Code("dryRun"), t(" 字段")),
      doc[1],
    )
    val list = doc[2] as MdBlock.ListBlock
    assertEquals(false, list.ordered)
    assertEquals(2, list.items.size)
    assertEquals(listOf(p(t("详见 "), MdInline.Link("https://example.com/docs", listOf(t("文档"))))), list.items[1])
    val table = doc[4] as MdBlock.Table
    assertTrue(table.header.size == 2 && table.rows.size == 1)
    assertEquals(MdBlock.ListBlock(true, 1, listOf(listOf(p(t("删除了"))), listOf(p(t("README"))))), doc[5])
    assertEquals(MdBlock.Code("ts", "const x = 1"), doc[6])
  }

  /** `<br>` is a line break, and a paragraph of nothing but breaks adds no block, in the stream too */
  @Test fun htmlBreaks() {
    val br = MdInline.LineBreak
    assertEquals(listOf(p(t("a"), br, t("b"), br, t("c "), br, t("d"))), Markdown.parse("a<br>b<BR/>c <br />\nd"))
    val runaway = "完成。\n\n" + List(500) { "<br>" }.joinToString("\n\n") + "\n\n<b>不是换行</b>"
    assertEquals(listOf(p(t("完成。")), p(t("<b>不是换行</b>"))), Markdown.parse(runaway))
    val stream = MarkdownStream()
    stream.parse(runaway.take(400))
    assertEquals(Markdown.parse(runaway), stream.parse(runaway))
  }

  @Test fun streamingSafety() {
    // An unclosed fence runs to the end; unclosed delimiters stay literal
    assertEquals(listOf(MdBlock.Code(null, "let a")), Markdown.parse("```\nlet a"))
    assertEquals(listOf(p(t("**half"))), Markdown.parse("**half"))
    assertEquals(listOf(p(t("`open"))), Markdown.parse("`open"))
  }

  @Test fun dollarsStayText() {
    assertEquals(listOf(p(t("价格 \$5 和 \$10"))), Markdown.parse("价格 \$5 和 \$10"))
    assertEquals(listOf(MdBlock.Math("x^2")), Markdown.parse("\$\$\nx^2\n\$\$"))
  }

  @Test fun nestedList() {
    val inner = MdBlock.ListBlock(false, 1, listOf(listOf(p(t("b")))))
    assertEquals(
      listOf(MdBlock.ListBlock(false, 1, listOf(listOf(p(t("a")), inner), listOf(p(t("c")))))),
      Markdown.parse("- a\n  - b\n- c"),
    )
  }

  /** The streamed parse equals a whole parse at every length of a growing text, and after a rewrite */
  @Test fun streamMatchesWholeParse() {
    val docs = listOf(
      "## 标题\n\n段落一，`code` 和 **粗体**。\n第二行\n| a | b |\n|---|---|\n| 1 | 2 |\n\n结尾段落",
      "- 一\n- 二\n\n- 三，空行后仍是同一个列表\n  续行\n\n1. 新列表\n2. 第二项\n\n---\n\n> 引用\n惰性续行\n\n收尾",
      "前言\n\n```rust\nfn a() {}\n\nfn b() {}\n```\n\n\$\$\nx^2\n\$\$\n\n- a\n  - b\n- c\n\n| x |\n|:-:|\n| y |",
      "开头一段，没有空行\n- 列表打断段落\n3. 有序从三开始不打断\n# 标题打断\n正文",
      // A delimiter row that stops being one as its line grows: the table it made goes back into the paragraph
      "段落\n| a |\n|---|之后变成文字\n\n尾",
    )
    for (doc in docs) {
      val stream = MarkdownStream()
      for (end in 1..doc.length) {
        val text = Remend.repair(doc.substring(0, end))
        if (stream.parse(text) != Markdown.parse(text)) fail("diverged at $end of ${doc.take(12)}")
      }
      // A rewrite, not an extension: the shared lines are reused only where equal
      val rewritten = doc.reversed()
      assertEquals(Markdown.parse(rewritten), stream.parse(rewritten))
      assertEquals(Markdown.parse(doc), stream.parse(doc))
    }
  }
}

/** Mirrors apps/ios LogicTests.swift StreamPaceTests */
class StreamPaceTest {
  private fun cut(text: String, n: Int, streaming: Boolean = true) = StreamCut.cut(Graphemes.of(text), n, streaming)
  private fun endsOnSyntax(text: String, n: Int) = StreamCut.endsOnSyntax(Graphemes.of(text), n)

  @Test fun cutsSkipMarkdownSyntax() {
    assertEquals(5, cut("- **节奏**很稳", 3))
    assertEquals(4, cut("ab cd", 3))
    assertTrue(endsOnSyntax("x\n12.", 5))
    assertTrue(endsOnSyntax("12", 2))
    assertTrue(!endsOnSyntax("ab12", 4))
    assertEquals(5, cut("hello **", 8))
    assertEquals(8, cut("hello **", 8, streaming = false))
  }

  @Test fun revealsAtMeasuredPace() {
    val pacer = StreamPacer(streaming = true)
    pacer.receive("字".repeat(100), streaming = true, now = 0.0)
    assertEquals(0, pacer.count)
    var now = 0.0
    repeat(10) { now += 0.032; pacer.tick(now, 0.032) }
    assertTrue(pacer.count in 21..99, "count ${pacer.count}")
    assertTrue(pacer.reveals.isNotEmpty())
    assertTrue(pacer.reveals.all { it.from < it.to })
    pacer.receive("字".repeat(100), streaming = false, now = now)
    repeat(20) { now += 0.032; pacer.tick(now, 0.032) }
    assertEquals(100, pacer.count)
    pacer.expire(now + 1)
    assertTrue(pacer.reveals.isEmpty())
  }

  @Test fun replacedTextShowsAtOnce() {
    val pacer = StreamPacer(streaming = true)
    pacer.receive("first draft", streaming = true, now = 0.0)
    pacer.receive("rewritten", streaming = true, now = 0.1)
    assertEquals("rewritten", pacer.visible)
  }

  @Test fun skippedTextShowsAtOnceAndResumesPacing() {
    val pacer = StreamPacer(streaming = true)
    pacer.receive("already on screen", streaming = true, now = 0.0, skip = true)
    assertEquals("already on screen", pacer.visible)
    assertTrue(!pacer.pending && pacer.reveals.isEmpty())
    pacer.receive("already on screen, and now streaming live", streaming = true, now = 1.0)
    assertEquals("already on screen", pacer.visible)
    assertTrue(pacer.pending)
    pacer.tick(1.032, 0.032)
    assertTrue(pacer.count > "already on screen".length)
  }

  @Test fun fadeFollowsRevealSteps() {
    val fade = StreamFade(10, listOf(StreamReveal(1.0, 4, 10)))
    assertEquals(0f, fade.opacity(0, 1.0))
    assertEquals(1f, fade.opacity(6, 1.0))
    assertEquals(1f, fade.opacity(0, 1 + StreamPace.GLYPH_FADE))
    assertEquals(6, fade.reach)
  }
}
