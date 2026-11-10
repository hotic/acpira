import AcpiraDesign
import AcpiraModels
import SwiftUI

// chat/PlanDocument.tsx: a saved plan as its own card between the work before it and the continuation after it. The
// card is the plan itself (no header or status word; the leading `# Title` names it for accessibility only), then an
// action bar while it can still be built: the execution model with the extra approvals behind one chip, and Build.
// Revising is a message: the composer's send answers the plan's revise option (Dock / AppModel.send).

/// What the plan card reads from its session (PlanDocumentContext). Equatable, so an environment value rebuilt from
/// each push's freshly decoded controls reads as unchanged when they are
struct PlanContext: Equatable {
  var controls = SessionControls()
  var running = false
  /// The session can take a build (status ready)
  var ready = false
}

extension EnvironmentValues {
  @Entry var planContext = PlanContext()
}

private extension Metrics {
  /// --plan-body-max at phone height: the card shows the plan's opening and fades out; the whole plan is one tap
  /// away in the code sheet, since a scroll view nested in the thread fights the thread's own scrolling
  static var planBody: CGFloat { WebMetrics.row * 10 }
}

struct PlanDocumentCard: View {
  let block: PlanDocumentBlock
  /// The approval the agent waits on for this plan, when it asks for one
  var permission: PermissionBlock?
  @Environment(\.planContext) private var ctx
  @Environment(\.sessionActions) private var actions
  @Environment(\.initialPopup) private var initialPopup
  /// The executor picked on the card; it applies only with Build, so the model still writing the plan stays
  @State private var selected: String?
  @State private var natural: CGFloat = 0

  var body: some View {
    let heading = planHeading(block)
    VStack(alignment: .leading, spacing: 0) {
      if !heading.body.isEmpty { planBody(heading.body, title: heading.title) }
      if block.status != "approved" && block.status != "executing" { bar() }
    }
    .frame(maxWidth: .infinity, alignment: .leading)
    // The dock cards' glass: a plan waiting to be built asks for an answer as the question card does
    .modifier(CardSurface())
    .accessibilityElement(children: .contain)
    .accessibilityLabel(Text(web: heading.title))
  }

  /// The plan's opening at the card's inset, faded at the cap; a tap opens the whole plan with a copy button
  private func planBody(_ text: String, title: String) -> some View {
    let clipped = natural > Metrics.planBody
    return StreamProse(markdown: text, streaming: false)
      .fixedSize(horizontal: false, vertical: true)
      .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { natural = $0 }
      .frame(maxHeight: Metrics.planBody, alignment: .top)
      .clipped()
      // The cut fades the plan itself out, since glass has no ground colour to fade to; the way to the rest sits in
      // the space the fade leaves clear
      .mask {
        VStack(spacing: 0) {
          Rectangle()
          if clipped {
            LinearGradient(colors: [.black, .clear], startPoint: .top, endPoint: .bottom).frame(height: Metrics.scrollFade)
            Color.clear.frame(height: WebType.text3.lineHeight + Metrics.s1)
          }
        }
      }
      .overlay(alignment: .bottom) {
        if clipped {
          Text(web: t("plan.expand"))
            .textStyle(WebType.text3)
            .foregroundStyle(Palette.popFg2)
            .frame(maxWidth: .infinity)
            .allowsHitTesting(false)
        }
      }
      .padding(Metrics.pad)
      .frame(maxWidth: .infinity, alignment: .leading)
      .codeViewer(title: title, copy: block.markdown, copyLabel: t("mobile.plan.copy")) {
        StreamProse(markdown: text, streaming: false)
      }
  }

  /// The actions under a top hairline: the executor chip, then Build as the card's one primary action
  private func bar() -> some View {
    let executor = PlanExecutor(controls: ctx.controls, selected: selected)
    let busy = !ctx.ready || (ctx.running && permission == nil) || block.status == "draft" || block.status == "executing"
    let canBuild = !busy && !block.markdown.isEmpty
    let primary = permission?.options.first { $0.kind == .allow_once }
    let revise = permission.flatMap(reviseOption)
    let extra = permission?.options.filter { $0.id != primary?.id && $0.id != revise?.id } ?? []
    return HStack(spacing: Metrics.gap) {
      Spacer(minLength: 0)
      if executor.control != nil || !extra.isEmpty {
        menu(executor, extra: extra, canBuild: canBuild)
      }
      Button(web: primary.map { permissionOption($0, plan: true).label } ?? t("plan.build")) {
        Haptics.play(.approve)
        actions.buildPlan(block.id, primary?.id, executor.pick)
      }
      .modifier(CardButton(primary: true))
      .disabled(!canBuild)
    }
    // The question card's footer: no hairline, the buttons inset like the card's text
    .padding(.horizontal, Metrics.pad)
    .padding(.bottom, Metrics.pad)
    .task(id: canBuild) { await probe(canBuild: canBuild, executor: executor, primary: primary) }
  }

  /// The executor chip: model families (a family with several variants opens its variants), then the approvals beyond
  /// Build. An allow option builds with the chosen executor; any other answers the approval as it is
  private func menu(_ executor: PlanExecutor, extra: [PermissionOption], canBuild: Bool) -> some View {
    let families = executor.control.map { ModelCatalog.group($0.options) } ?? []
    return PopupMenu {
      if !families.isEmpty {
        Section(t("plan.executor")) {
          ForEach(families, id: \.key) { family in
            if family.variants.count > 1 {
              Menu {
                ForEach(family.variants, id: \.id) { variant in
                  pick(variantName(variant), on: variant.id == executor.value) { selected = variant.id }
                }
              } label: {
                checked(family.name, on: family.key == executor.family?.key)
              }
            } else if let only = family.variants.first {
              pick(family.name, on: only.id == executor.value) { selected = only.id }
            }
          }
        }
      }
      if !extra.isEmpty {
        Section(t("plan.moreApprovals")) {
          ForEach(extra) { option in
            let allow = option.kind == .allow_once || option.kind == .allow_always
            Button(web: permissionOption(option, plan: true).label) {
              if allow {
                actions.buildPlan(block.id, option.id, executor.pick)
              } else if let permission {
                actions.permission(permission.id, option.id)
              }
            }
            .disabled(allow ? !canBuild : !ctx.ready)
          }
        }
      }
    } label: {
      if let family = executor.family {
        ChipLabel(family.name, meta: executor.meta) { VendorMark(brand: family.brand, name: family.name, size: Metrics.icon) }
      } else {
        ChipLabel(executor.label ?? t("plan.approvals"), meta: executor.meta)
      }
    }
    .modifier(CardButton(primary: false, fixed: false))
    .menuStyle(.button)
    .lineLimit(1)
    .accessibilityLabel(Text(web: t("plan.approvalsAria")))
  }

  private func pick(_ label: String, on: Bool, action: @escaping () -> Void) -> some View {
    Button(action: action) { checked(label, on: on) }
  }

  @ViewBuilder
  private func checked(_ label: String, on: Bool) -> some View {
    if on { Label { Text(web: label) } icon: { Image(systemName: "checkmark") } } else { Text(web: label) }
  }

  /// `-popup plan.build` (debug builds): Build is pressed once it can be, for a live check without scripted taps;
  /// `plan.build.model` first picks the executor's last family, `plan.revise` sends a revision from the composer
  private func probe(canBuild: Bool, executor: PlanExecutor, primary: PermissionOption?) async {
    #if DEBUG
    guard canBuild, let popup = initialPopup, popup.hasPrefix("plan."), !PlanProbe.claimed else { return }
    PlanProbe.claimed = true
    try? await Task.sleep(for: .seconds(2))
    switch popup {
    case "plan.revise":
      actions.send("Revise: keep the plan to one step", [])
    case "plan.build.model":
      let last = executor.control.flatMap { ModelCatalog.group($0.options).last?.variants.first }
      var pick = executor.pick
      if let last, let control = executor.control { pick = ModelPick(configId: control.id, value: last.id) }
      actions.buildPlan(block.id, primary?.id, pick)
    default:
      actions.buildPlan(block.id, primary?.id, executor.pick)
    }
    #endif
  }
}

/// `-popup plan.*`: whether a card has already taken the launch's one probe
enum PlanProbe {
  nonisolated(unsafe) static var claimed = false
}
