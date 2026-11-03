import AcpiraModels
import Foundation
import SwiftUI
#if canImport(UIKit)
import UIKit
#endif

/// shared/pastedText.ts: only a paste is classified, never typing, dictation or a restored draft.
nonisolated enum PastedText {
  static let characters = 2000
  static let lines = 20
  static let maxBytes = 1024 * 1024

  enum Result: Equatable {
    case inline
    case attachment(Attachment)
    case tooBig
  }

  static func collect(_ text: String, name: String) -> Result {
    // JavaScript counts UTF-16 units; CRLF is one line break, as are a lone CR and LF.
    let units = text.utf16
    if units.count < characters {
      var count = 1
      var previous: UInt16?
      for unit in units {
        if unit == 13 || (unit == 10 && previous != 13) { count += 1 }
        if count >= lines { break }
        previous = unit
      }
      if count < lines { return .inline }
    }
    let bytes = Data(text.utf8)
    guard bytes.count <= maxBytes else { return .tooBig }
    // Like a staged photo, a local text attachment carries base64 until the engine gives it a blob name.
    return .attachment(.text(blob: bytes.base64EncodedString(), name: name))
  }

  static func inline(_ blob: String?) -> String? {
    guard let blob, !blob.contains("."), let data = Data(base64Encoded: blob) else { return nil }
    return String(data: data, encoding: .utf8)
  }
}

extension View {
  /// Hooks the native paste event before a large payload reaches the field's text layout.
  func attachLongPastes(to staged: Binding<[Attachment]>, notice: @escaping (String) -> Void) -> some View {
    #if canImport(UIKit)
    background(TextPasteProbe { text in
      let name = t("attach.pastedText")
      switch PastedText.collect(text, name: name) {
      case .inline: return false
      case .attachment(let attachment): staged.wrappedValue.append(attachment)
      case .tooBig: notice(t("attach.tooBigFile", ["name": name, "mb": PastedText.maxBytes >> 20]))
      }
      return true
    }.allowsHitTesting(false).accessibilityHidden(true))
    #else
    self
    #endif
  }
}

#if canImport(UIKit)
/// Leaves SwiftUI in charge of the field, its selection and keyboard. Only its native paste delegate is wrapped.
private struct TextPasteProbe: UIViewRepresentable {
  var take: (String) -> Bool

  func makeUIView(context: Context) -> Finder { Finder(take: take) }
  func updateUIView(_ view: Finder, context: Context) {
    view.take = take
    view.install()
  }
  static func dismantleUIView(_ view: Finder, coordinator: ()) { view.detach() }

  final class Finder: UIView, UITextPasteDelegate {
    var take: (String) -> Bool
    private weak var input: (UIView & UITextPasteConfigurationSupporting)?
    private weak var previous: (any UITextPasteDelegate)?

    init(take: @escaping (String) -> Bool) {
      self.take = take
      super.init(frame: .zero)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    override func didMoveToWindow() {
      super.didMoveToWindow()
      if window == nil { detach() } else { install() }
    }

    func install() {
      // The background and native field are siblings below SwiftUI's field host. Never search the whole window:
      // the expanded editor can coexist with the composer's field underneath it.
      Task { @MainActor [weak self] in
        guard let self, window != nil else { return }
        var parent = superview
        while let view = parent, !(view is UIWindow) {
          if let field = find(in: view) {
            if input !== field { detach(); input = field }
            if field.pasteDelegate !== self {
              previous = field.pasteDelegate
              field.pasteDelegate = self
            }
            return
          }
          parent = view.superview
        }
      }
    }

    private func find(in view: UIView) -> (UIView & UITextPasteConfigurationSupporting)? {
      if let field = view as? UITextField { return field }
      if let field = view as? UITextView { return field }
      for child in view.subviews where child !== self {
        if let field = find(in: child) { return field }
      }
      return nil
    }

    func detach() {
      if input?.pasteDelegate === self { input?.pasteDelegate = previous }
      input = nil
      previous = nil
    }

    func textPasteConfigurationSupporting(_ textPasteConfigurationSupporting: any UITextPasteConfigurationSupporting,
                                          transform item: any UITextPasteItem) {
      // UIKit loads the user's paste, not an unsolicited clipboard read. Discarding the item leaves even an
      // existing selection untouched; returning an empty string would instead delete the selected text.
      guard item.itemProvider.canLoadObject(ofClass: NSString.self) else {
        fallback(textPasteConfigurationSupporting, item: item)
        return
      }
      item.itemProvider.loadObject(ofClass: NSString.self) { [weak self] object, _ in
        let text = object as? String
        Task { @MainActor in
          guard let self else { item.setDefaultResult(); return }
          if let text, self.take(text) { item.setNoResult() }
          else { self.fallback(textPasteConfigurationSupporting, item: item) }
        }
      }
    }

    private func fallback(_ input: any UITextPasteConfigurationSupporting, item: any UITextPasteItem) {
      if let previous, previous.responds(to: #selector(UITextPasteDelegate.textPasteConfigurationSupporting(_:transform:))) {
        previous.textPasteConfigurationSupporting?(input, transform: item)
      } else {
        item.setDefaultResult()
      }
    }
  }
}
#endif
