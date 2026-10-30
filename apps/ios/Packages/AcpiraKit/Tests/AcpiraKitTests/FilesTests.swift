import AcpiraModels
import Foundation
import Testing
@testable import AcpiraUI

// The review pages' logic: file links read as packages/ui/test/fileLinks.test.ts reads them, the file page's rows as
// FileViewer.tsx viewRows lays them out, and the plural forms the counts take

@Suite struct FileLinkTests {
  @Test(arguments: [
    (".agents/knowledge/npcs/overview.md", FileLink(path: ".agents/knowledge/npcs/overview.md")),
    ("src/foo.ts", FileLink(path: "src/foo.ts")),
    ("./src/foo.ts", FileLink(path: "./src/foo.ts")),
    ("../foo.ts", FileLink(path: "../foo.ts")),
    ("/repo/a.ts", FileLink(path: "/repo/a.ts")),
    ("foo.ts:12", FileLink(path: "foo.ts", line: 12)),
    ("src/a.ts#L12", FileLink(path: "src/a.ts", line: 12)),
    ("src/a.ts#L12-20", FileLink(path: "src/a.ts", line: 12)),
    ("overview.md", FileLink(path: "overview.md")),
    (#"C:\repo\a.ts:27"#, FileLink(path: #"C:\repo\a.ts"#, line: 27)),
    ("file:///repo/src/a.ts", FileLink(path: "/repo/src/a.ts")),
    ("file:///repo/my%20file.ts", FileLink(path: "/repo/my file.ts")),
    ("file:///repo/src/a.ts#L9", FileLink(path: "/repo/src/a.ts", line: 9)),
    ("file:///C:/repo/a.ts", FileLink(path: "C:/repo/a.ts")),
    ("file:///C:/repo/a%23L12", FileLink(path: "C:/repo/a#L12")),
  ])
  func accepts(_ text: String, _ link: FileLink) {
    #expect(FileLinks.parse(text) == link)
  }

  @Test(arguments: ["https://example.com/a.ts", "http://example.com", "mailto:a@b.com", "javascript:alert(1)", "e.g.",
                    "hello world", "#footnote-1", "www.example.com", "not a path", ""])
  func rejects(_ text: String) {
    #expect(FileLinks.parse(text) == nil)
  }

  @Test func ownLinkRoundTrips() throws {
    let link = FileLink(path: "/repo/my file#1.ts", line: 40)
    let url = try #require(FileLinks.url(link))
    #expect(url.scheme == "acpira-file")
    #expect(FileLinks.decode(url) == link)
    #expect(FileLinks.parse(url.absoluteString) == link)
  }
}

@Suite struct FileRowsTests {
  @Test func removedLinesStandBeforeTheirReplacement() {
    // Line 2 replaced by two lines, line 4 removed at the end
    let rows = fileLines("a\nB1\nB2\nc\n", hunks: [ProjectHunk(at: 2, add: 2, del: ["b"]), ProjectHunk(at: 5, add: 0, del: ["d"])])
    #expect(rows.map(\.kind) == [.ctx, .del, .add, .add, .ctx, .del])
    #expect(rows.map(\.code) == ["a", "b", "B1", "B2", "c", "d"])
    #expect(rows.map(\.newLine) == [1, nil, 2, 3, 4, nil])
    #expect(rows.map(\.oldLine) == [1, 2, nil, nil, 3, 4])
    #expect(changeStarts(rows) == [1, 5])
  }

  @Test func crlfAndNoHunks() {
    let rows = fileLines("x\r\ny", hunks: [])
    #expect(rows.map(\.code) == ["x", "y"])
    #expect(changeStarts(rows).isEmpty)
  }

  @Test func pathParts() {
    #expect(fileName("src/ui/App.tsx") == "App.tsx")
    #expect(folderOf("src/ui/App.tsx") == "src/ui")
    #expect(folderOf("README.md") == "")
  }

  @Test func markdownPreview() {
    #expect(isMarkdownPath("docs/A.MD") && isMarkdownPath("x.mdx") && !isMarkdownPath("md") && !isMarkdownPath("a.ts"))
    // Front matter turns into a YAML block; a rule later in the file stays
    #expect(markdownPreviewText("---\nname: x\n---\n# T\n---\n") == "```yaml\nname: x\n```\n\n# T\n---\n")
    #expect(markdownPreviewText("# T\n---\nname: x\n---\n") == "# T\n---\nname: x\n---\n")
    #expect(joinPath("docs/dev", "../a.md") == "docs/a.md")
    #expect(joinPath("/w/docs", "./b/c.md") == "/w/docs/b/c.md")
    #expect(joinPath("", "../a.md") == nil)
    #expect(joinPath("docs", "/abs/a.md") == nil)
  }
}

@Suite @MainActor struct PluralTests {
  @Test func countsPickTheirForm() {
    #expect(L10n.pluralCategory("ru", 1) == "one")
    #expect(L10n.pluralCategory("ru", 3) == "few")
    #expect(L10n.pluralCategory("ru", 11) == "many")
    #expect(L10n.pluralCategory("ru", 22) == "few")
    #expect(L10n.pluralCategory("en", 1) == "one")
    #expect(L10n.pluralCategory("en", 2) == nil)
    #expect(L10n.pluralCategory("fr", 0) == "one")
    #expect(L10n.pluralCategory("zh-CN", 1) == nil)
  }
}

// A diff card's copy (codeDiff.ts diffCopyText) and where the diff sits for the engine to find its files
@Suite struct DiffCopyTests {
  private let lines = [DiffLine(kind: .hunk, text: "@@ -1,2 +1,2 @@"), DiffLine(kind: .ctx, text: " a"),
                       DiffLine(kind: .del, text: "-b"), DiffLine(kind: .add, text: "+B")]

  @Test func copyTakesTheNewFileElseTheVisibleNewCode() {
    #expect(diffCopyText(lines, nil) == "a\nB")
    #expect(diffCopyText(lines, .omitted(path: "x.ts")) == "a\nB")
    #expect(diffCopyText(lines, .full(FullDiffSource(path: "x.ts", oldText: "a\nb\nc", newText: "a\nB\nc"))) == "a\nB\nc")
  }

  @Test func theLocationCountsOnlyTheCallsDiffs() throws {
    let other = [DiffLine(kind: .add, text: "+z")]
    let json = #"{"id":"e1","kind":"edit","verb":"Edit","status":"completed","contents":[{"type":"text","text":"note"},"#
      + #"{"type":"diff","lines":[{"kind":"add","text":"+z"}]},{"type":"diff","lines":[{"kind":"ctx","text":" a"}]}]}"#
    let block = try JSONDecoder().decode(ToolCallBlock.self, from: Data(json.utf8))
    #expect(diffLocation(block, other) == DiffLocation(toolCallId: "e1", nth: 0))
    #expect(diffLocation(block, [DiffLine(kind: .ctx, text: " a")]) == DiffLocation(toolCallId: "e1", nth: 1))
    #expect(diffLocation(block, lines) == nil)
  }
}
