import AcpiraDesign
import AcpiraModels
import SwiftUI

// chat/Questions.tsx: the open question card docked above the composer, one question per page, lettered options,
// an Other field, Skip on the left and Next / Submit on the right of the footer, as system glass buttons.

struct QuestionCard: View {
  let block: QuestionBlock
  var onAnswer: ([String: QuestionAnswer], Bool) -> Void = { _, _ in }

  private struct Pick { var options: [String] = []; var other = false; var text = "" }
  @State private var picks: [String: Pick] = [:]
  @State private var page = 0
  @FocusState private var otherFocused: Bool
  @Environment(\.snapshot) private var snapshot
  @Environment(\.room) private var room
  private static let keys = Array("ABCDEFGHIJKLMNOPQRSTUVWXYZ").map(String.init)

  var body: some View {
    let qs = block.questions
    let index = min(page, max(0, qs.count - 1))
    if qs.indices.contains(index) {
      let current = qs[index]
      let last = index == qs.count - 1
      let answers = collected(qs)
      let canSubmit = !answers.isEmpty && qs.allSatisfy { $0.required != true || answers[$0.id] != nil }
      let canAdvance = last ? canSubmit : (current.required != true || answers[current.id] != nil)

      VStack(spacing: 0) {
        Scroller(.vertical) {
          VStack(alignment: .leading, spacing: Metrics.questionSectionGap) {
            Row(dense: true, leadTop: true) {
              Icon(.messageCircleQuestion)
            } content: {
              Text(web: current.text)
                .textStyle(TypeScale.text2, weight: .medium)
                .foregroundStyle(Palette.fgStrong)
                .fixedSize(horizontal: false, vertical: true)
            } trailing: {
              Text(web: t("question.of", ["n": index + 1, "total": qs.count]))
            }
            if let message = block.message, index == 0 {
              Text(web: message).textStyle(TypeScale.text2).foregroundStyle(Palette.popFg2)
            }
            options(current)
          }
          .padding(.horizontal, Metrics.pad)
          .padding(.top, Metrics.questionTopPad)
          .padding(.bottom, Metrics.questionSectionGap)
        }
        .scrollBounceBehavior(.basedOnSize)
        // --question-body-max: min(--row × 8, 35dvh)
        .frame(maxHeight: room.height(Metrics.row * 8))
        .fixedSize(horizontal: false, vertical: true)

        HStack(spacing: Metrics.gap) {
          Button(web: t("question.skip")) { onAnswer(answers, true) }
            .modifier(CardButton(primary: false))
          Spacer(minLength: 0)
          if index > 0 {
            Button(web: t("question.prev")) { page = index - 1 }
              .modifier(CardButton(primary: false))
          }
          Button(web: t(last ? "question.submit" : "question.next")) {
            if last { onAnswer(answers, false) } else { page = index + 1 }
          }
          .modifier(CardButton(primary: true))
          .disabled(!canAdvance)
        }
        .padding(.horizontal, Metrics.pad)
        .padding(.top, Metrics.s1)
        .padding(.bottom, Metrics.pad)
      }
      .modifier(CardSurface())
    }
  }

  @ViewBuilder
  private func options(_ q: Question) -> some View {
    let pick = picks[q.id] ?? Pick()
    VStack(alignment: .leading, spacing: 0) {
      if q.kind == .multiple {
        Text(web: t("question.multiple")).textStyle(TypeScale.text3).foregroundStyle(Palette.popFg2)
          .padding(.bottom, Metrics.questionSectionGap)
      }
      ForEach(Array(q.options.enumerated()), id: \.element.id) { i, option in
        let selected = pick.options.contains(option.id)
        Button { choose(q, option.id) } label: {
          OptionRow(key: Self.keys[safe: i] ?? "\(i + 1)", selected: selected) {
            VStack(alignment: .leading, spacing: Metrics.questionDescriptionGap) {
              Text(web: option.label).foregroundStyle(Palette.fg1)
              if let description = option.description {
                Text(web: description).textStyle(TypeScale.text3).foregroundStyle(Palette.popFg2)
              }
            }
            .fixedSize(horizontal: false, vertical: true)
          }
        }
        .buttonStyle(OptionButtonStyle(selected: selected))
      }
      if q.kind != .text && q.other == true {
        // A one-line field centres in the row; top alignment left the row's spare height all below it
        OptionRow(key: Self.keys[safe: q.options.count] ?? "", selected: pick.other, leadTop: false) {
          Group {
            if snapshot {
              // A snapshot draws the placeholder where the field would be (ImageRenderer cannot draw a TextField)
              Text(web: pick.text.isEmpty ? t("question.other") : pick.text)
                .foregroundStyle(pick.text.isEmpty ? Palette.popFg2 : Palette.fg1)
                .frame(maxWidth: .infinity, alignment: .leading)
            } else {
              TextField(text: binding(q)) {
                Text(web: t("question.other")).foregroundStyle(Palette.popFg2)
              }
              .focused($otherFocused)
            }
          }
          .textStyle(TypeScale.text2)
          .foregroundStyle(Palette.fg1)
          .frame(height: Metrics.lead)
        }
        .modifier(OptionFill(on: pick.other))
      }
      if q.kind == .text {
        let placeholder = t(q.numeric == true ? "question.numberPlaceholder" : "question.textPlaceholder")
        Group {
          if snapshot {
            Text(web: pick.text.isEmpty ? placeholder : pick.text)
              .foregroundStyle(pick.text.isEmpty ? Palette.popFg3 : Palette.fg1)
              .frame(maxWidth: .infinity, alignment: .leading)
          } else {
            TextField(text: binding(q)) { Text(web: placeholder).foregroundStyle(Palette.popFg3) }
          }
        }
        .textStyle(TypeScale.text2)
        .padding(.horizontal, Metrics.gap)
        .frame(height: Metrics.ctl)
        .overlay(RoundedRectangle(cornerRadius: Metrics.rMd).strokeBorder(Palette.lineStrong))
      }
    }
  }

  private func binding(_ q: Question) -> Binding<String> {
    Binding {
      picks[q.id]?.text ?? ""
    } set: { text in
      var p = picks[q.id] ?? Pick()
      p.text = text
      if q.kind != .text { p.other = true }
      if q.kind == .single { p.options = [] }
      picks[q.id] = p
    }
  }

  private func choose(_ q: Question, _ id: String) {
    var p = picks[q.id] ?? Pick()
    if q.kind == .multiple {
      if let at = p.options.firstIndex(of: id) { p.options.remove(at: at) } else { p.options.append(id) }
    } else {
      p.other = false; p.options = [id]
    }
    picks[q.id] = p
  }

  private func collected(_ qs: [Question]) -> [String: QuestionAnswer] {
    var out: [String: QuestionAnswer] = [:]
    for q in qs {
      let p = picks[q.id] ?? Pick()
      let text = p.text.trimmingCharacters(in: .whitespacesAndNewlines)
      switch q.kind {
      case .text: if !text.isEmpty { out[q.id] = .one(text) }
      case .single:
        if p.other { if !text.isEmpty { out[q.id] = .one(text) } } else if let first = p.options.first { out[q.id] = .one(first) }
      default:
        let list = p.options + (p.other && !text.isEmpty ? [text] : [])
        if !list.isEmpty { out[q.id] = .many(list) }
      }
    }
    return out
  }
}

/// A lettered option: the key tile (a check once picked) in the lead slot, the label column beside it
struct OptionRow<Content: View>: View {
  let key: String
  let selected: Bool
  /// Pin the key to the first text line (labels that may wrap); false centres it with a one-line field
  var leadTop = true
  @ViewBuilder let content: () -> Content

  var body: some View {
    Row(leadTop: leadTop) {
      // The key ring is as large as the header's glyph, so it reads as a lead mark, not a control; the lead slot keeps
      // its width, so labels stay on the header's text line
      Group {
        if selected {
          Icon(.check, size: WebMetrics.chevron, strokeWidth: 2)
        } else {
          Text(web: key).textStyle(TypeScale.optionKey, weight: .medium)
        }
      }
      .foregroundStyle(selected ? Palette.fg1 : Palette.popFg2)
      .frame(width: Metrics.icon, height: Metrics.icon)
      .overlay(Circle().strokeBorder(Palette.lineStrong))
      .frame(width: Metrics.lead, height: Metrics.lead)
    } content: {
      content().frame(maxWidth: .infinity, alignment: .leading)
    }
    // min-h-row on the padded option box: the row inside is --row less the option padding
    .environment(\.rowHeight, Metrics.row - 2 * Metrics.questionOptionPad)
  }
}

struct OptionButtonStyle: ButtonStyle {
  var selected: Bool
  func makeBody(configuration: Configuration) -> some View {
    configuration.label.modifier(OptionFill(on: selected || configuration.isPressed))
  }
}

/// An option's fill bleeds outward, so its text stays on the header's text line instead of the fill pushing it in.
/// Sideways it also clears the key box, which overhangs the lead's icon column
struct OptionFill: ViewModifier {
  var on: Bool
  private var bleed: CGFloat { Metrics.questionOptionPad + (Metrics.lead - WebMetrics.icon) / 2 }
  func body(content: Content) -> some View {
    let shape = RoundedRectangle(cornerRadius: Metrics.rMd)
    content
      .padding(.vertical, Metrics.questionOptionPad)
      .padding(.horizontal, bleed)
      .background(shape.fill(on ? Palette.active : .clear))
      .contentShape(shape)
      .padding(.horizontal, -bleed)
  }
}

/// The dock cards' surface: the system's Liquid Glass in the composer's variant (`GlassStyle.bar`), rounded like the
/// composer's card, so a card floating over the thread reads as the same material as the bar under it. Glass brings
/// its own edge and depth: no drawn line or shadow. A card holds paragraphs where the bar holds one line, so a veil of
/// the card colour lies over the glass; a tint on the glass itself took it down to the page's black on a phone
struct CardSurface: ViewModifier {
  func body(content: Content) -> some View {
    let shape = RoundedRectangle(cornerRadius: Metrics.rCardRound, style: .continuous)
    content
      .background(shape.fill(Palette.cardBg.opacity(Metrics.cardVeil)))
      .clipShape(shape)
      .glassSurface(GlassStyle.bar, in: shape)
  }
}

/// A dock card's action: the system's glass capsule; the way on is prominent in the send button's ink, and falls
/// back to plain glass with faint text while it cannot run, so a disabled step never reads as a lit one
struct CardButton: ViewModifier {
  var primary: Bool
  /// One line at its own width; off for a chip that truncates when the row is short
  var fixed = true
  @Environment(\.isEnabled) private var enabled
  func body(content: Content) -> some View {
    let lit = primary && enabled
    let label = content.textStyle(TypeScale.text2, weight: lit ? .medium : .regular)
    // As the web's `h-ctl` buttons: a squeezed footer truncates its chip, never wraps a button
    if lit {
      label.glassButtonStyle(prominent: true).tint(Palette.fgStrong).foregroundStyle(Palette.bg0)
        .fixedSize(horizontal: fixed, vertical: false)
    } else {
      label.glassButtonStyle().foregroundStyle(enabled ? Palette.fg1 : Palette.popFg3)
        .fixedSize(horizontal: fixed, vertical: false)
    }
  }
}

/// A dock card's corner control (close, retry): a small glass circle, as a system sheet's close button
struct CardCornerButton: View {
  let icon: LucideIcon
  let label: String
  var action: () -> Void
  var body: some View {
    Button(action: action) {
      Icon(icon, size: Metrics.icon, strokeWidth: 1.75)
        .foregroundStyle(Palette.fg1)
        .frame(width: Metrics.icon, height: Metrics.icon)
    }
    .glassButtonStyle(circle: true)
    .buttonBorderShape(.circle)
    .accessibilityLabel(Text(web: label))
  }
}

extension Array {
  subscript(safe index: Int) -> Element? { indices.contains(index) ? self[index] : nil }
}
