import AcpiraDesign
import AcpiraModels
import SwiftUI

// Completion.tsx, Slash.tsx and Mention.tsx at phone size: the `/` command list and the `@` list (personas, then files)
// as one glass card over the composer. The card is part of the composer's own stack, above the staged attachments: a
// list hung outside the composer's frame over the transcript would lose its taps to the thread's scroll view.

/// One card of suggestions: a row per item, scrolling past `gooPanelRows`; `empty` stands in for no rows
struct SuggestionList<Item, Label: View>: View {
  var items: [Item]
  var empty: String?
  var key: (Item) -> String
  var pick: (Item) -> Void
  @ViewBuilder var label: (Item) -> Label

  var body: some View {
    let rows = CGFloat(max(1, items.count))
    ScrollView {
      VStack(spacing: 0) {
        if items.isEmpty, let empty {
          Text(web: empty, paragraph: false)
            .textStyle(TypeScale.text3)
            .foregroundStyle(Palette.fg3)
            .padding(.horizontal, Metrics.gooPadX)
            .frame(maxWidth: .infinity, minHeight: Metrics.gooRow, alignment: .leading)
        }
        ForEach(Array(items.enumerated()), id: \.offset) { _, item in
          Button { pick(item) } label: {
            HStack(spacing: Metrics.s2) { label(item) }
              .padding(.horizontal, Metrics.gooPadX)
              .frame(maxWidth: .infinity, minHeight: Metrics.gooRow, alignment: .leading)
              .contentShape(Rectangle())
          }
          .buttonStyle(SuggestionRowStyle())
          .id(key(item))
        }
      }
      .padding(.vertical, Metrics.s1_5)
    }
    .scrollIndicators(.hidden)
    .scrollBounceBehavior(.basedOnSize)
    .frame(height: min(rows, Metrics.gooPanelRows) * Metrics.gooRow + Metrics.s1_5 * 2)
    // Text over the transcript: the regular lens, as the toast's (the clear one let the thread read through the rows)
    .glassSurface(.regular, in: RoundedRectangle(cornerRadius: Metrics.rGooPanel, style: .continuous))
    .transition(.opacity.combined(with: .scale(scale: Metrics.gooSeed, anchor: .bottomLeading)))
  }
}

/// A row lit under the finger with the popup's pressed fill
struct SuggestionRowStyle: ButtonStyle {
  func makeBody(configuration: Configuration) -> some View {
    configuration.label
      .background(configuration.isPressed ? Palette.popHover : .clear)
  }
}

/// A row of the `/` list: a working mode the composer enters itself, or one of the agent's commands
enum SlashHit: Hashable {
  case mode(SessionOption)
  case command(SlashCommand)

  var key: String {
    switch self {
    case .mode(let m): "mode:\(m.id)"
    case .command(let c): c.name
    }
  }
}

/// SlashList's host action row: the mode's glyph, its name bright, one line on what it does faint after it
struct ModeRow: View {
  let mode: SessionOption
  var body: some View {
    Icon(modeIcon(mode), size: Metrics.icon, strokeWidth: 1.75).foregroundStyle(Palette.fg1)
    Text(web: modeName(mode), paragraph: false)
      .textStyle(TypeScale.text2)
      .foregroundStyle(Palette.fgStrong)
      .lineLimit(1)
      .layoutPriority(2)
    if let hint = modeHint(mode), !hint.isEmpty {
      Text(web: hint, paragraph: false)
        .textStyle(TypeScale.text3)
        .foregroundStyle(Palette.fg3)
        .lineLimit(1)
        .frame(maxWidth: .infinity, alignment: .leading)
    }
  }
}

/// The working mode in force, after the bar's chips (Composer.tsx ModePill): outlined where the chips are glass, so it
/// reads as a token and not a third picker. The whole pill leaves the mode (a phone's × alone is too small to hit)
struct ModePill: View {
  let mode: SessionOption
  let exit: () -> Void
  var body: some View {
    let name = modeName(mode)
    Button(action: exit) {
      HStack(spacing: Metrics.s1) {
        Icon(modeIcon(mode), size: Metrics.icon, strokeWidth: 1.75).foregroundStyle(Palette.fg1)
        Text(web: name, paragraph: false)
          .textStyle(TypeScale.text2)
          .foregroundStyle(Palette.fg1)
          .lineLimit(1)
          .fixedSize()
        Icon(.x, size: Metrics.chevron, strokeWidth: 1.75).foregroundStyle(Palette.fg3)
      }
      .padding(.leading, Metrics.s2)
      .padding(.trailing, Metrics.s1_5)
      .frame(height: Metrics.rowDense)
      .overlay(Capsule().strokeBorder(Palette.conversationLine))
      // The bar's height takes the tap, not just the outline
      .frame(height: Metrics.barCtl)
      .contentShape(Rectangle())
    }
    .buttonStyle(.plain)
    .accessibilityLabel(Text(web: t("composer.modeExit", ["mode": name])))
  }
}

/// SlashList's row: the glyph, `/name` bright, the input hint faint beside it, the description trailing
struct CommandRow: View {
  let command: SlashCommand
  var body: some View {
    Icon(.squareSlash, size: Metrics.icon, strokeWidth: 1.5).foregroundStyle(Palette.fg3)
    Text(web: "/\(command.name)", paragraph: false)
      .textStyle(TypeScale.mono)
      .foregroundStyle(Palette.fgStrong)
      .lineLimit(1)
      .layoutPriority(2)
    if let hint = command.input?.hint, !hint.isEmpty {
      Text(web: hint, paragraph: false)
        .textStyle(TypeScale.mono)
        .foregroundStyle(Palette.fg3)
        .lineLimit(1)
    }
    if !command.description.isEmpty {
      Text(web: command.description, paragraph: false)
        .textStyle(TypeScale.text3)
        .foregroundStyle(Palette.fg3)
        .lineLimit(1)
        .frame(maxWidth: .infinity, alignment: .leading)
    }
  }
}

/// MentionList's row: a persona (its CLI's mark, the name, the CLI and model faint after it) or a file (the name bright,
/// its folder faint)
struct MentionRow: View {
  let hit: FileHit
  let persona: MentionPersona?

  var body: some View {
    if let persona {
      SummonMark(persona: persona, size: Metrics.icon).foregroundStyle(Palette.fg2)
      Text(web: persona.name, paragraph: false)
        .textStyle(TypeScale.text2)
        .foregroundStyle(Palette.fgStrong)
        .lineLimit(1)
        .layoutPriority(2)
      if let cli = persona.cli {
        Text(web: cli, paragraph: false).textStyle(TypeScale.text3).foregroundStyle(Palette.fg2).lineLimit(1).layoutPriority(1)
      }
      if let model = persona.model {
        Text(web: model, paragraph: false).textStyle(TypeScale.text3).foregroundStyle(Palette.fg3).lineLimit(1)
      }
      if let effort = persona.effort {
        Text(web: effort, paragraph: false).textStyle(TypeScale.text3).foregroundStyle(Palette.fg3).lineLimit(1)
      }
    } else {
      let cut = hit.path.lastIndex(of: "/")
      Icon(imageFile(hit.path) ? .image : .fileText, size: Metrics.icon, strokeWidth: 1.5).foregroundStyle(Palette.fg3)
      Text(web: cut.map { String(hit.path[hit.path.index(after: $0)...]) } ?? hit.path, paragraph: false)
        .textStyle(TypeScale.mono)
        .foregroundStyle(Palette.fgStrong)
        .lineLimit(1)
        .layoutPriority(2)
      if let cut {
        Text(web: String(hit.path[..<cut]), paragraph: false)
          .textStyle(TypeScale.text3)
          .foregroundStyle(Palette.fg3)
          .lineLimit(1)
          .truncationMode(.head)
      }
    }
  }

  /// attachments.ts imageMimeOf, by extension
  private func imageFile(_ path: String) -> Bool {
    let ext = path.split(separator: ".").last.map { $0.lowercased() } ?? ""
    return ["png", "jpg", "jpeg", "gif", "webp", "heic", "bmp", "svg"].contains(ext)
  }
}

/// What leads a summonable entry (Mention.tsx SummonMark): its CLI's mark, or the people glyph for a group
struct SummonMark: View {
  let persona: MentionPersona
  var size: CGFloat
  var body: some View {
    if persona.group {
      Icon(.users, size: size, strokeWidth: 1.5)
    } else {
      VendorMark(brand: persona.agent, name: persona.cli ?? persona.name, size: size)
    }
  }
}

extension EnvironmentValues {
  /// The personas the composer offers for `@` summons (settings → Subagents); none on screens without a session
  @Entry var mentionPersonas: [MentionPersona] = []
  /// The auto-compact threshold the context ring marks
  @Entry var compactAt: Int? = nil
}
