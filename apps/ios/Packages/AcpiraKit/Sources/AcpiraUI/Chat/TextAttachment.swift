import AcpiraDesign
import SwiftUI
#if canImport(UIKit)
import UIKit
#endif

/// The same text attachment in the draft, queue and transcript: a pill opens its original contents first.
struct TextAttachmentChip: View {
  var blob: String?
  var name: String
  var onRemove: (() -> Void)? = nil
  @State private var open = false

  var body: some View {
    AttachmentPill(name: name, icon: .fileText, onOpen: { open = true }, onRemove: onRemove)
      .sheet(isPresented: $open) { TextAttachmentSheet(blob: blob, name: name) }
  }
}

private struct TextAttachmentSheet: View {
  var blob: String?
  var name: String
  @Environment(\.media) private var media
  @State private var text: String?
  @State private var failed = false
  @State private var preview = false

  var body: some View {
    CodeSheet(title: name, copy: text ?? "", copyLabel: t("common.copy"), wrapInScrollView: false,
              detents: [.large], copyEnabled: text != nil, markdownPreview: text == nil ? nil : $preview) {
      Group {
        if let text {
          if preview {
            ScrollView(.vertical) {
              Prose(markdown: markdownPreviewText(text))
                .frame(maxWidth: .infinity, alignment: .leading)
                .textSelection(.enabled)
            }
          } else {
            AttachmentText(text: text)
          }
        } else if failed {
          Text(web: t("attach.loadFailed", ["name": name])).foregroundStyle(Palette.fg3)
        } else {
          ProgressView(t("attach.loading"))
        }
      }
      .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
    .task {
      if let inline = PastedText.inline(blob) {
        text = inline
      } else if let blob, let session = media.session {
        do { text = try await media.loader.text(session: session, blob: blob) }
        catch { failed = true }
      } else {
        failed = true
      }
    }
  }
}

#if canImport(UIKit)
/// One bounded native text viewport: even a megabyte is selectable without building one SwiftUI view per line.
private struct AttachmentText: UIViewRepresentable {
  let text: String

  func makeUIView(context: Context) -> UITextView {
    let view = UITextView()
    view.isEditable = false
    view.isSelectable = true
    view.backgroundColor = .clear
    view.textContainerInset = .zero
    view.textContainer.lineFragmentPadding = 0
    view.alwaysBounceHorizontal = false
    view.showsHorizontalScrollIndicator = false
    return view
  }

  func updateUIView(_ view: UITextView, context: Context) {
    let ink = UIColor(Palette.fgStrong)
    if view.text != text || view.textColor != ink {
      let style = NSMutableParagraphStyle()
      style.minimumLineHeight = TypeScale.body.lineHeight
      style.maximumLineHeight = TypeScale.body.lineHeight
      view.attributedText = NSAttributedString(string: text, attributes: [
        .font: UIFont(name: "InterVariable", size: TypeScale.body.size) ?? .systemFont(ofSize: TypeScale.body.size),
        .foregroundColor: ink,
        .paragraphStyle: style,
      ])
    }
  }
}
#else
private struct AttachmentText: View {
  let text: String
  var body: some View {
    ScrollView {
      Text(verbatim: text).textStyle(TypeScale.body).foregroundStyle(Palette.fgStrong)
        .frame(maxWidth: .infinity, alignment: .leading).textSelection(.enabled)
    }
  }
}
#endif
