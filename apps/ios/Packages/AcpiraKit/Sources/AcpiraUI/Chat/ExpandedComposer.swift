import AcpiraDesign
import SwiftUI

// The composer's whole-screen editor, as ChatGPT's expand corner opens one: a long draft is written and read over the
// whole page instead of inside the card's few lines. It edits the same draft, so collapsing returns to the card with
// the text as it stands; the chips (attachments, model, mode) stay in the card.

/// The draft over the whole screen: collapse in the bar, send above the keyboard
struct ExpandedComposer: View {
  @Binding var text: String
  /// Something to send: text or staged attachments
  var filled: Bool
  var onSend: () -> Void
  var onCollapse: () -> Void
  @FocusState private var focused: Bool

  var body: some View {
    NavigationStack {
      TextEditor(text: $text)
        .focused($focused)
        .textStyle(TypeScale.field)
        .foregroundStyle(Palette.fgStrong)
        .scrollContentBackground(.hidden)
        .padding(.horizontal, Metrics.pad)
        .background(Palette.bg0)
        .toolbar {
          ToolbarItem(placement: .primaryAction) {
            Button(action: onCollapse) {
              Icon(.minimize2, size: Metrics.iconCtl)
            }
            .accessibilityLabel(Text(web: t("mobile.composer.collapse")))
          }
        }
        // Send rides the keyboard's top edge, at the corner it holds in the card
        .safeAreaInset(edge: .bottom, spacing: 0) {
          HStack(spacing: 0) {
            Spacer(minLength: 0)
            SendButton(running: false, filled: filled, action: onSend)
          }
          .padding(.horizontal, Metrics.pad)
          .padding(.vertical, Metrics.s2)
        }
    }
    .onAppear { focused = true }
  }
}

/// Raises the whole-screen editor: a full-screen cover on the phone, a sheet where `swift test` builds for macOS
struct FullEditorCover<Editor: View>: ViewModifier {
  @Binding var open: Bool
  var onDismiss: () -> Void
  @ViewBuilder var editor: () -> Editor

  func body(content: Content) -> some View {
    #if os(iOS)
    content.fullScreenCover(isPresented: $open, onDismiss: onDismiss, content: editor)
    #else
    content.sheet(isPresented: $open, onDismiss: onDismiss, content: editor)
    #endif
  }
}
