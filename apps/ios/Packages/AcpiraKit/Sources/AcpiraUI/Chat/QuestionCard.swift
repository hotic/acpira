import AcpiraDesign
import AcpiraModels
import SwiftUI

// chat/Questions.tsx: the open question card docked above the composer, one question per page, lettered options,
// an Other field, Skip on the left and Next / Submit on the right of a hairline footer.

struct QuestionCard: View {
  let block: QuestionBlock
  var onAnswer: ([String: QuestionAnswer], Bool) -> Void = { _, _ in }

  private struct Pick { var options: [String] = []; var other = false; var text = "" }
  @State private var picks: [String: Pick] = [:]
  @State private var page = 0
  @FocusState private var otherFocused: Bool
  @Environment(\.snapshot) private var snapshot
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
              Text(web: message).textStyle(TypeScale.text2).foregroundStyle(Palette.fg2)
            }
            options(current)
          }
          .padding(.horizontal, Metrics.pad)
          .padding(.top, Metrics.questionTopPad)
          .padding(.bottom, Metrics.questionSectionGap)
        }
        .scrollBounceBehavior(.basedOnSize)
        // --question-body-max: min(--row × 8, 35dvh)
        .frame(maxHeight: Metrics.row * 8)
        .fixedSize(horizontal: false, vertical: true)

        HStack(spacing: Metrics.gap) {
          Button(web: t("question.skip")) { onAnswer(answers, true) }
            .buttonStyle(QuestionButtonStyle(primary: false, enabled: true))
          Spacer(minLength: 0)
          if index > 0 {
            Button(web: t("question.prev")) { page = index - 1 }
              .buttonStyle(QuestionButtonStyle(primary: false, enabled: true))
          }
          Button(web: t(last ? "question.submit" : "question.next")) {
            if last { onAnswer(answers, false) } else { page = index + 1 }
          }
          .buttonStyle(QuestionButtonStyle(primary: true, enabled: canAdvance))
          .disabled(!canAdvance)
        }
        .padding(.horizontal, Metrics.pad)
        .padding(.vertical, Metrics.questionSectionGap)
        // border-t: the hairline takes its own pixel above the footer's padding
        .padding(.top, Metrics.hairline)
        .overlay(alignment: .top) { Rectangle().fill(Palette.line).frame(height: Metrics.hairline) }
      }
      .modifier(CardSurface())
    }
  }

  @ViewBuilder
  private func options(_ q: Question) -> some View {
    let pick = picks[q.id] ?? Pick()
    VStack(alignment: .leading, spacing: 0) {
      if q.kind == .multiple {
        Text(web: t("question.multiple")).textStyle(TypeScale.text3).foregroundStyle(Palette.fg2)
          .padding(.bottom, Metrics.questionSectionGap)
      }
      ForEach(Array(q.options.enumerated()), id: \.element.id) { i, option in
        let selected = pick.options.contains(option.id)
        Button { choose(q, option.id) } label: {
          OptionRow(key: Self.keys[safe: i] ?? "\(i + 1)", selected: selected) {
            VStack(alignment: .leading, spacing: Metrics.questionDescriptionGap) {
              Text(web: option.label).foregroundStyle(Palette.fg1)
              if let description = option.description {
                Text(web: description).textStyle(TypeScale.text3).foregroundStyle(Palette.fg2)
              }
            }
            .fixedSize(horizontal: false, vertical: true)
          }
        }
        .buttonStyle(OptionButtonStyle(selected: selected))
      }
      if q.kind != .text && q.other == true {
        OptionRow(key: Self.keys[safe: q.options.count] ?? "", selected: pick.other) {
          Group {
            if snapshot {
              // A snapshot draws the placeholder where the field would be (ImageRenderer cannot draw a TextField)
              Text(web: pick.text.isEmpty ? t("question.other") : pick.text)
                .foregroundStyle(pick.text.isEmpty ? Palette.fg2 : Palette.fg1)
                .frame(maxWidth: .infinity, alignment: .leading)
            } else {
              TextField(text: binding(q)) {
                Text(web: t("question.other")).foregroundStyle(Palette.fg2)
              }
              .focused($otherFocused)
            }
          }
          .textStyle(TypeScale.text2)
          .foregroundStyle(Palette.fg1)
          .frame(height: Metrics.lead)
        }
        .padding(Metrics.questionOptionPad)
        .background(RoundedRectangle(cornerRadius: Metrics.rMd).fill(pick.other ? Palette.active : .clear))
      }
      if q.kind == .text {
        let placeholder = t(q.numeric == true ? "question.numberPlaceholder" : "question.textPlaceholder")
        Group {
          if snapshot {
            Text(web: pick.text.isEmpty ? placeholder : pick.text)
              .foregroundStyle(pick.text.isEmpty ? Palette.fg3 : Palette.fg1)
              .frame(maxWidth: .infinity, alignment: .leading)
          } else {
            TextField(text: binding(q)) { Text(web: placeholder).foregroundStyle(Palette.fg3) }
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
  @ViewBuilder let content: () -> Content

  var body: some View {
    Row(leadTop: true) {
      Group {
        if selected { Icon(.check, size: Metrics.icon, strokeWidth: 1.75) } else { Text(web: key).textStyle(TypeScale.text3) }
      }
      .foregroundStyle(selected ? Palette.fg1 : Palette.fg2)
      .frame(width: Metrics.lead, height: Metrics.lead)
      .overlay(RoundedRectangle(cornerRadius: Metrics.rSm).strokeBorder(Palette.lineStrong))
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
    configuration.label
      .padding(Metrics.questionOptionPad)
      .background(RoundedRectangle(cornerRadius: Metrics.rMd).fill(selected || configuration.isPressed ? Palette.active : .clear))
      .contentShape(RoundedRectangle(cornerRadius: Metrics.rMd))
  }
}

/// The footer's --ctl-sm buttons: secondary on a half-strength hairline, primary dimmed to text while it cannot advance
struct QuestionButtonStyle: ButtonStyle {
  var primary: Bool
  var enabled: Bool
  func makeBody(configuration: Configuration) -> some View {
    let lit = primary && enabled
    configuration.label
      .textStyle(TypeScale.text2, weight: lit ? .medium : .regular)
      .foregroundStyle(lit ? Palette.fgStrong : primary ? Palette.fg3 : Palette.fg2)
      // px-3 inside a 1 px border
      .padding(.horizontal, Metrics.s3 + Metrics.hairline)
      .frame(height: Metrics.ctlSm)
      .background(RoundedRectangle(cornerRadius: Metrics.rMd).fill(lit ? Palette.active : configuration.isPressed ? Palette.hover : .clear))
      .overlay(RoundedRectangle(cornerRadius: Metrics.rMd).strokeBorder(lit ? Palette.lineStrong : Palette.line.opacity(0.5)))
  }
}

/// The `card` surface under the tonal axis: rounded-lg, --card-line, --card-bg and the card shadow
struct CardSurface: ViewModifier {
  func body(content: Content) -> some View {
    // A card over the composer rounds like it
    let shape = RoundedRectangle(cornerRadius: Metrics.rCardRound, style: .continuous)
    content
      // `border`: the 1 px card line sits outside the content box
      .padding(Metrics.hairline)
      .clipShape(shape)
      .background(shape.fill(Palette.cardBg).shadow(color: .black.opacity(0.3), radius: 1, y: 1))
      .overlay(shape.strokeBorder(Palette.cardLine))
  }
}

extension Array {
  subscript(safe index: Int) -> Element? { indices.contains(index) ? self[index] : nil }
}
