import AcpiraMarkdown
import Testing

// The markdown subset the transcript renders, including half-streamed input
struct MarkdownTests {
  @Test func blocks() {
    let doc = Markdown.parse("""
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
    """)
    #expect(doc.count == 7)
    #expect(doc[0] == .heading(level: 2, [.text("改动总结")]))
    #expect(doc[1] == .paragraph([.strong([.text("核心变更")]), .text("："), .code("parseArgs"), .text(" 新增 "), .code("dryRun"), .text(" 字段")]))
    guard case .list(false, _, let items) = doc[2] else { Issue.record("not a list"); return }
    #expect(items.count == 2)
    #expect(items[1] == [.paragraph([.text("详见 "), .link(url: "https://example.com/docs", [.text("文档")])])])
    guard case .table(_, let header, let rows) = doc[4] else { Issue.record("not a table"); return }
    #expect(header.count == 2 && rows.count == 1)
    #expect(doc[5] == .list(ordered: true, start: 1, items: [[.paragraph([.text("删除了")])], [.paragraph([.text("README")])]]))
    #expect(doc[6] == .code(lang: "ts", text: "const x = 1"))
  }

  @Test func streamingSafety() {
    // An unclosed fence runs to the end; unclosed delimiters stay literal
    #expect(Markdown.parse("```\nlet a") == [.code(lang: nil, text: "let a")])
    #expect(Markdown.parse("**half") == [.paragraph([.text("**half")])])
    #expect(Markdown.parse("`open") == [.paragraph([.text("`open")])])
  }

  @Test func dollarsStayText() {
    #expect(Markdown.parse("价格 $5 和 $10") == [.paragraph([.text("价格 $5 和 $10")])])
    #expect(Markdown.parse("$$\nx^2\n$$") == [.math("x^2")])
  }

  @Test func nestedList() {
    let doc = Markdown.parse("- a\n  - b\n- c")
    #expect(doc == [.list(ordered: false, start: 1, items: [
      [.paragraph([.text("a")]), .list(ordered: false, start: 1, items: [[.paragraph([.text("b")])]])],
      [.paragraph([.text("c")])],
    ])])
  }

  /// The streamed parse keeps settled blocks and parses the tail again: at every length of a growing text, and after a
  /// rewrite, it equals a whole parse. The documents put block ends where the deciding line comes late (a paragraph a
  /// table interrupts only once its delimiter row arrives, or no longer once that row grows into text, a list a blank
  /// line does not end, an unclosed fence)
  @Test func streamMatchesWholeParse() {
    let docs = [
      "## 标题\n\n段落一，`code` 和 **粗体**。\n第二行\n| a | b |\n|---|---|\n| 1 | 2 |\n\n结尾段落",
      "- 一\n- 二\n\n- 三，空行后仍是同一个列表\n  续行\n\n1. 新列表\n2. 第二项\n\n---\n\n> 引用\n惰性续行\n\n收尾",
      "前言\n\n```rust\nfn a() {}\n\nfn b() {}\n```\n\n$$\nx^2\n$$\n\n- a\n  - b\n- c\n\n| x |\n|:-:|\n| y |",
      "开头一段，没有空行\n- 列表打断段落\n3. 有序从三开始不打断\n# 标题打断\n正文",
      // A delimiter row that stops being one as its line grows: the table it made goes back into the paragraph
      "段落\n| a |\n|---|之后变成文字\n\n尾",
    ]
    for doc in docs {
      var stream = MarkdownStream()
      let chars = Array(doc)
      for end in 1...chars.count {
        let text = Remend.repair(String(chars[..<end]))
        let streamed = stream.parse(text)
        let whole = Markdown.parse(text)
        if streamed != whole { Issue.record("diverged at \(end) of \(doc.prefix(12))"); break }
      }
      // A rewrite, not an extension: the shared lines are reused only where equal
      let rewritten = String(doc.reversed())
      #expect(stream.parse(rewritten) == Markdown.parse(rewritten))
      #expect(stream.parse(doc) == Markdown.parse(doc))
    }
  }
}
