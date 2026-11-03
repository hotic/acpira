import AcpiraModels
import Foundation
import Testing
@testable import AcpiraUI

@MainActor struct PastedTextTests {
  private let name = "Pasted text.txt"

  @Test func shortPastesStayInTheField() {
    for text in ["", "你好", "/compact", String(repeating: "a", count: 1999),
                 Array(repeating: "line", count: 19).joined(separator: "\n"), String(repeating: "😀", count: 999)] {
      #expect(PastedText.collect(text, name: name) == .inline)
    }
  }

  @Test func longPastesTravelVerbatimAsTextDrafts() throws {
    let texts = [String(repeating: "中", count: 2000), String(repeating: "😀", count: 1000)]
      + ["\n", "\r\n", "\r"].map { Array(repeating: "  内容  ", count: 20).joined(separator: $0) }
    for text in texts {
      guard case .attachment(let attachment) = PastedText.collect(text, name: name) else {
        Issue.record("Expected a text attachment"); continue
      }
      #expect(AppModel.drafts([attachment]) == [.text(name: name, text: text)])
      guard case .text(let blob, _, _) = attachment else { continue }
      #expect(PastedText.inline(blob) == text)
    }
    // Stored attachment ids must be fetched through the session, never interpreted as inline bytes.
    #expect(PastedText.inline("abcdef1234567890.txt") == nil)
    #expect(PastedText.inline(nil) == nil)
  }

  /// Paste, queue, edit, send again: the queue row names the engine's stored blob, which is read back before the prompt
  /// returns to the composer, so the resend still carries the text
  @Test func queuedPasteSurvivesAnEditAndResend() async throws {
    let text = String(repeating: "长文本\n", count: 30)
    let stored: [AcpiraModels.Attachment] = [.text(blob: "0123456789abcdef.txt", name: name), .quote(text: "q", comment: nil)]
    #expect(AppModel.drafts(stored) == [.quote(text: "q")])
    let local = try await AppModel.localCopies(stored) { blob in
      #expect(blob == "0123456789abcdef.txt")
      return Data(text.utf8)
    }
    #expect(AppModel.drafts(local) == [.text(name: name, text: text), .quote(text: "q")])

    struct Gone: Error {}
    await #expect(throws: Gone.self) { _ = try await AppModel.localCopies(stored) { _ in throw Gone() } }
  }

  @Test func byteLimitMatchesTheDesktop() {
    if case .attachment = PastedText.collect(String(repeating: "a", count: PastedText.maxBytes), name: name) {} else {
      Issue.record("The exact byte limit should be accepted")
    }
    #expect(PastedText.collect(String(repeating: "a", count: PastedText.maxBytes + 1), name: name) == .tooBig)
    #expect(PastedText.collect(String(repeating: "中", count: PastedText.maxBytes / 3 + 1), name: name) == .tooBig)
  }
}
