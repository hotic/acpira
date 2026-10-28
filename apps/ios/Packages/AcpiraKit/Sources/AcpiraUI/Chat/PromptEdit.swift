import AcpiraDesign
import AcpiraModels
import SwiftUI

// HistoryMessage.tsx's inline editor on the phone. The desktop swaps the prompt card for a composer in place; on a phone
// the keyboard covers that spot, so the chat's own composer takes the prompt in instead, as messengers edit a sent
// message, and a line above it says what is being edited and cancels.

/// The line above the composer while a sent prompt is being rewritten: what it was, and ✕ to give the composer back
struct EditingBar: View {
  let original: UserTurn
  var onCancel: () -> Void

  var body: some View {
    Row {
      Icon(.pencil, size: Metrics.icon)
    } content: {
      Text(web: t("mobile.edit.title"))
        .textStyle(TypeScale.text2, weight: .medium)
        .foregroundStyle(Palette.fg1)
        .lineLimit(1)
        .fixedSize()
      if let first = firstLine(original.text) {
        Text(web: first)
          .textStyle(TypeScale.text2)
          .foregroundStyle(Palette.fg3)
          .lineLimit(1)
          .truncationMode(.tail)
          .frame(minWidth: 0, alignment: .leading)
          .layoutPriority(-1)
      }
    } trailing: {
      IconButton(.x, label: t("history.cancel"), action: onCancel)
    }
    .padding(.horizontal, Metrics.pad)
    .padding(.vertical, Metrics.s1)
    .background(RoundedRectangle(cornerRadius: Metrics.rMd).fill(Palette.cmpBg))
    .overlay(RoundedRectangle(cornerRadius: Metrics.rMd).strokeBorder(Palette.conversationLine))
  }
}

/// The session as the editing composer reads it (HistoryEditor's composer props): the edit's own selections, nothing
/// running to stop, and no context ring (the edit replaces the context the ring measures)
func editingView(_ session: SessionView, _ edit: TurnEdit) -> SessionView {
  var view = session
  view.controls = edit.controls
  view.running = false
  view.usage = nil
  return view
}
