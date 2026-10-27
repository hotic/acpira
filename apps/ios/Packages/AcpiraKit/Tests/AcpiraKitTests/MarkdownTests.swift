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
}
