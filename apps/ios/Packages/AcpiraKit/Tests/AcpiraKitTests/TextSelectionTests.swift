#if canImport(UIKit)
import AcpiraDesign
import AcpiraMarkdown
import AcpiraModels
import Foundation
import SwiftUI
import Testing
import UIKit
@testable import AcpiraUI

@MainActor @Suite(.serialized) struct TextSelectionTests {
  @Test func copyingCJKOmitsOnlyGeneratedBreaks() {
    let source = "中文原文\u{200B}含有\u{2060}字符 👨‍👩‍👧‍👦"
    let text = selectionContent(source, style: WebType.text1)
    #expect(text.string != source)
    #expect(selectionText(text, range: NSRange(location: 0, length: text.length)) == source)
    let range = (text.string as NSString).range(of: "中\u{200B}文")
    #expect(selectionText(text, range: range) == "中文")
  }

  @Test func copyingInlineMarkdownKeepsSourceTextWithoutCodePadding() {
    let inlines: [MDInline] = [.text("运行 "), .code("src/main.swift"), .text("，再"), .strong([.text("检查")]),
                               .link(url: "https://example.com", [.text("结果")])]
    let builder = InlineBuilder(style: WebType.text1, ink: .white, strongInk: .white, accent: .blue)
    let text = builder.selectionParagraph(inlines, weight: .regular)
    #expect(selectionText(text, range: NSRange(location: 0, length: text.length)) == Markdown.plain(inlines))
    let range = (text.string as NSString).range(of: "main.swift")
    #expect(selectionText(text, range: range) == "main.swift")
  }

  @Test func selectionOverlayDoesNotChangeTheParagraphsLayout() throws {
    let text = "中文消息保留原来的宽度与换行，支持选择其中一段文字。"
    let plain = Text(web: text).textStyle(WebType.text1).fixedSize(horizontal: false, vertical: true)
    let proposal = CGSize(width: 240, height: UIView.layoutFittingExpandedSize.height)
    let before = UIHostingController(rootView: plain).sizeThatFits(in: proposal)
    let host = UIHostingController(rootView: plain.selectableText(text, style: WebType.text1))
    let after = host.sizeThatFits(in: proposal)
    #expect(abs(before.width - after.width) < 0.5)
    #expect(abs(before.height - after.height) < 0.5)
  }

  @Test func previewStartsAtTouchedLineWithoutChangingTheCopySource() throws {
    let source = "开头的内容\n中间的内容\n末尾的内容"
    let view = SelectionTextView(frame: CGRect(x: 0, y: 0, width: 300, height: 200))
    view.isEditable = false
    view.isScrollEnabled = false
    view.textContainerInset = .zero
    view.textContainer.lineFragmentPadding = 0
    view.attributedText = selectionContent(source, style: WebType.text1)
    view.layoutIfNeeded()
    let range = (view.attributedText.string as NSString).range(of: "末")
    let start = try #require(view.position(from: view.beginningOfDocument, offset: range.location))
    let end = try #require(view.position(from: start, offset: 1))
    let textRange = try #require(view.textRange(from: start, to: end))
    let rect = view.firstRect(for: textRange)
    _ = view.hitTest(CGPoint(x: rect.midX, y: rect.midY), with: nil)
    #expect(view.previewText == "末尾的内容")
    #expect(selectionText(view.attributedText, range: NSRange(location: 0, length: view.attributedText.length)) == source)
  }

  @Test func promptLeavesLongPressToMessageActionsUntilSelectionIsRequested() async throws {
    let text = "Edit this whole historical message, not just the selected word."
    let scope = TextSelectionScope()
    let host = UIHostingController(rootView: TextSelectionSurface(text: selectionContent(text, style: WebType.text1))
      .environment(\.textSelectionScope, scope))
    let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 360, height: 200))
    window.rootViewController = host
    window.makeKeyAndVisible()
    defer { window.isHidden = true; window.rootViewController = nil }
    host.view.layoutIfNeeded()
    let view = try #require(selectionView(in: host.view))
    #expect(view.isUserInteractionEnabled)
    let longPress = UILongPressGestureRecognizer()
    #expect(!view.gestureRecognizerShouldBegin(longPress))
    #expect(scope.target === view)

    scope.select()
    try await Task.sleep(for: .milliseconds(100))
    #expect(view.isUserInteractionEnabled)
    #expect(view.selectedRange == NSRange(location: 0, length: view.attributedText.length))
    #expect(scope.active)
    #expect(view.gestureRecognizerShouldBegin(longPress))

    view.resignFirstResponder()
    try await Task.sleep(for: .milliseconds(100))
    #expect(!scope.active)
  }

  @Test func promptSelectionActionsEditTheWholeHistoricalTurn() throws {
    let prompt = UserTurn(text: "Original historical prompt")
    var edited: Int?
    var objective: String?
    var actions = SessionActions()
    actions.editPrompt = { edited = $0 }
    actions.goal = { _, value in objective = value }
    let host = UIHostingController(rootView: UserMessageCard(turn: prompt, index: 4, editable: true, goal: true)
      .environment(\.sessionActions, actions))
    let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 360, height: 200))
    window.rootViewController = host
    window.makeKeyAndVisible()
    defer { window.isHidden = true; window.rootViewController = nil }
    host.view.layoutIfNeeded()
    let view = try #require(selectionView(in: host.view))
    #expect(!view.gestureRecognizerShouldBegin(UILongPressGestureRecognizer()))
    try #require(view.actions.first { $0.title == t("history.edit") }).run("historical")
    #expect(edited == 4)
    try #require(view.actions.first { $0.title == t("goal.setFromPrompt") }).run("historical")
    #expect(objective == prompt.text)
  }

  private func selectionView(in view: UIView) -> SelectionTextView? {
    if let text = view as? SelectionTextView { return text }
    return view.subviews.lazy.compactMap { selectionView(in: $0) }.first
  }

  @Test func nativeLinksKeepFileAndWebDestinations() throws {
    let builder = InlineBuilder(style: WebType.text1, ink: .white, strongInk: .white, accent: .blue)
    let text = builder.selectionParagraph([.code("src/main.swift:12"), .text(" "),
      .link(url: "https://example.com", [.text("website")])], weight: .regular)
    let fileRange = (text.string as NSString).range(of: "main.swift:12")
    let file = try #require(text.attribute(.link, at: fileRange.location, effectiveRange: nil) as? URL)
    #expect(FileLinks.decode(file) == FileLink(path: "src/main.swift", line: 12))
    let webRange = (text.string as NSString).range(of: "website")
    #expect((text.attribute(.link, at: webRange.location, effectiveRange: nil) as? URL)?.absoluteString == "https://example.com")
  }
}
#endif
