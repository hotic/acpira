import AcpiraModels
import Foundation
import Testing
@testable import AcpiraUI

// The highlighter end to end: the bundled shiki runs in JavaScriptCore and answers with the webview's colours
// (test/ios-shiki.test.ts keeps the bundle in step with the webview's shiki)
struct SyntaxTests {
  @Test func languages() {
    #expect(codeLanguage("src/cli/index.ts") == "typescript")
    #expect(codeLanguage("a.d.ts") == "typescript")
    #expect(codeLanguage("README.MD") == "markdown")
    #expect(codeLanguage(".gitignore") == nil)
    #expect(codeLanguage("a..ts") == nil)
    #expect(codeLanguage("Makefile") == nil)
    #expect(fenceLanguage("shell") == "shellscript")
    #expect(fenceLanguage(" TS ") == "typescript")
    #expect(fenceLanguage("text") == nil)
    #expect(fenceLanguage(nil) == nil)
  }

  @Test func tokens() throws {
    let lines = try #require(SyntaxHighlighter.shared.tokenize("const a = 'x';\nlet b", language: "typescript", theme: .dark))
    #expect(lines.count == 2)
    // dark-plus: keywords #569CD6, strings #CE9178; the default foreground is left to the card's ink, and whitespace
    // joins the run before it
    #expect(lines[0].first == CodeToken(text: "const ", color: "#569CD6"))
    #expect(lines[0].contains(CodeToken(text: "'x'", color: "#CE9178")))
    #expect(lines[0].allSatisfy { $0.color?.uppercased() != "#D4D4D4" })
    #expect(lines.map { $0.map(\.text).joined() } == ["const a = 'x';", "let b"])
    #expect(SyntaxHighlighter.shared.tokenize("x", language: nil, theme: .dark) == nil)
  }

  @Test func cachedDiffNeverTokenizesMissingSegments() throws {
    let highlighter = SyntaxHighlighter()
    let first = [DiffLine(kind: .add, text: "+const first = 1")]
    let missing = [DiffLine(kind: .hunk, text: "@@"), DiffLine(kind: .add, text: "+const unseen = 2")]
    #expect(highlighter.cachedDiff(first, path: "x.ts", theme: .dark) == nil)
    let expected = try #require(highlighter.diff(first, path: "x.ts", theme: .dark))
    #expect(highlighter.cachedDiff(first, path: "x.ts", theme: .dark) == expected)
    #expect(highlighter.cachedDiff(first + missing, path: "x.ts", theme: .dark) == nil)
    #expect(highlighter.cached("const unseen = 2", language: "typescript", theme: .dark) == nil)
  }

  @Test func diffSides() throws {
    // A deleted line takes the old side's tokens and never shifts the new side's grammar: the old side's open comment
    // must not colour the added line as a comment
    let lines = try JSONDecoder().decode([DiffLine].self, from: Data("""
    [{"kind":"ctx","text":" let a = 1","oldLine":1,"newLine":1},{"kind":"del","text":"-/* gone","oldLine":2},
     {"kind":"add","text":"+let b = 2","newLine":2}]
    """.utf8))
    let rows = try #require(SyntaxHighlighter.shared.diff(lines, path: "x.ts", theme: .dark))
    #expect(rows.count == 3)
    #expect(rows[1]?.first?.color == "#6A9955")
    #expect(rows[2]?.first == CodeToken(text: "let ", color: "#569CD6"))
    #expect(SyntaxHighlighter.shared.diff(lines, path: "x.txt", theme: .dark) == nil)
  }
}
