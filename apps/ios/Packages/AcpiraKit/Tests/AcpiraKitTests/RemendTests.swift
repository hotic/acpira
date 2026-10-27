import AcpiraMarkdown
import Testing

/// The Swift remend port against the installed remend's own output (RemendCases.generated.swift)
struct RemendTests {
  @Test func streamedPrefixesMatchRemend() {
    #expect(RemendCases.documents.map(\.cases.count).reduce(0, +) > 1000)
    for (text, cases) in RemendCases.documents {
      let units = Array(text.utf16)
      for (i, expected) in cases.enumerated() {
        // A prefix that splits a surrogate pair never streams
        if i > 0, UTF16.isLeadSurrogate(units[i - 1]) { continue }
        let prefix = String(decoding: units[..<i], as: UTF16.self)
        let want: String
        switch expected {
        case .append(let tail): want = prefix + tail
        case .full(let full): want = full
        }
        #expect(Remend.repair(prefix) == want, "prefix \(i): \(prefix.debugDescription)")
      }
    }
  }

  @Test func edgesMatchRemend() {
    // The edges exercise every repair, not just pass-through
    #expect(RemendCases.edges.filter { $0.0 != $0.1 }.count > 30)
    for (input, output) in RemendCases.edges {
      #expect(Remend.repair(input) == output, "\(input.debugDescription)")
    }
  }

  @Test func streamingProseClosesTheTail() {
    // The streaming fixture's last line: the unclosed code span renders as code, not as a stray backtick
    let blocks = Markdown.parse(Remend.repair("- b.ts → `parseA"))
    guard case .list(_, _, let items) = blocks.first, case .paragraph(let inlines) = items.first?.first else {
      Issue.record("not a list: \(blocks)")
      return
    }
    #expect(inlines.contains(.code("parseA")))
  }
}
