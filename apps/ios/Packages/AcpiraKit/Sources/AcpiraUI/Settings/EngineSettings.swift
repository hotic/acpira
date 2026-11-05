import AcpiraDesign
import AcpiraModels
import SwiftUI

// The engine's settings the phone offers (General.tsx, PluginsPage.tsx, AgentPage.tsx), as system form rows: the
// agents (switches, order, the default), how sessions run (steering, compaction, account switching), and per agent the
// Plan-mode approval and which models its menus show. Model sources, keys, MCP and the rest stay on the computer:
// typing a provider key on the phone widens what a lost phone exposes.

/// The pages the settings push
enum SettingsRoute: Hashable {
  case agent(AgentId)
  case models(AgentId)
  case developer

  /// The pages down to a launch argument's page
  static func launchPath(_ arg: String?) -> NavigationPath {
    guard let arg else { return NavigationPath() }
    let parts = arg.split(separator: ":", maxSplits: 1).map(String.init)
    let routes: [SettingsRoute] = switch (parts.first, parts.count > 1 ? parts[1] : nil) {
    case ("agent", let id?): [.agent(id)]
    case ("models", let id?): [.agent(id), .models(id)]
    case ("developer", _): [.developer]
    default: []
    }
    return NavigationPath(routes)
  }
}

/// The settings sheet's sections for the engine's settings, between the app's own and the connection
struct EngineSettingsSections: View {
  @Environment(AppModel.self) private var model

  var body: some View {
    if let settings = model.settings {
      let agents = model.settingsAgents
      // Under the agents' header, its own card above the list: a row without a mark among the marked ones would break
      // their column
      Section {
        // Switched-off agents are not offered; a default that was switched off reads as the agent new sessions fall
        // back to (pickDefaultAgent)
        Picker(t("settings.defaultAgent"), selection: Binding(
          get: { pickDefaultAgent(model.agents, settings.defaultAgent) },
          set: { model.setSetting(\.defaultAgent, "defaultAgent", $0) })) {
          ForEach(launchable(agents)) { a in Text(verbatim: a.name).tag(a.id) }
        }
      } header: {
        Text(web: t("mobile.settings.agents"))
      }
      // Every agent laid out, as the system's Settings lists its apps: a row opens that agent's page (its on / off
      // switch included) in one tap, and touch and hold drags it into the order every list follows
      Section {
        ForEach(agents) { a in
          NavigationLink(value: SettingsRoute.agent(a.id)) { AgentRow(agent: a) }
        }
        .onMove { model.moveAgents(from: $0, to: $1) }
      } footer: {
        Text(web: t("mobile.settings.agents.desc"))
      }
      Section {
        // The menu first, the switches under it; the threshold stays under the switch it depends on
        Picker(t("settings.accountSwitch"), selection: Binding(
          get: { settings.accountSwitch },
          set: { model.setSetting(\.accountSwitch, "accountSwitch", $0) })) {
          ForEach(["off", "earliestReset", "mostRemaining", "listOrder"], id: \.self) { s in
            Text(web: t("settings.accountSwitch.\(s)")).tag(s)
          }
        }
        // Titles alone: the desktop's hints run to three lines at the phone's width
        Toggle(isOn: Binding(get: { settings.steerQueued }, set: { model.setSetting(\.steerQueued, "steerQueued", $0) })) {
          Text(web: t("settings.steerQueued"), paragraph: false)
        }
        .tint(.green)
        Toggle(isOn: Binding(get: { settings.autoCompact }, set: { model.setSetting(\.autoCompact, "autoCompact", $0) })) {
          Text(web: t("settings.autoCompact"), paragraph: false)
        }
        .tint(.green)
        if settings.autoCompact {
          // Presets in the menu instead of a number field; a value set on the computer stays listed
          Picker(t("settings.compactAt"), selection: Binding(
            get: { settings.compactAtTokens },
            set: { model.setSetting(\.compactAtTokens, "compactAtTokens", $0) })) {
            ForEach(compactPresets(settings.compactAtTokens), id: \.self) { n in
              Text(verbatim: n == 0 ? t("settings.compactAt.model") : "\(n / 1000)\(t("settings.compactAt.unit"))").tag(n)
            }
          }
        }
      } header: {
        Text(web: t("mobile.settings.sessions"))
      } footer: {
        Text(web: t("mobile.settings.engine.desc"))
      }
    }
  }

  /// 0 first (settings.ts COMPACT_AT_MODEL: each session's own model window), then MIN_COMPACT_AT_TOKENS and up, the
  /// current value kept in its place (a count typed on the desktop); the top preset matches the 1M windows
  private func compactPresets(_ current: Int) -> [Int] {
    Array(Set([0, 100_000, 200_000, 300_000, 400_000, 600_000, 800_000, 1_000_000, current])).sorted()
  }
}

/// A row's title with its one-line hint under it, as the system's settings word a switch that needs a word more
struct SettingLabel: View {
  let title: String
  var hint: String?
  var body: some View {
    VStack(alignment: .leading, spacing: Metrics.s0_5) {
      Text(web: title, paragraph: false)
      if let hint {
        Text(web: hint, paragraph: false).textStyle(TypeScale.text2).foregroundStyle(Palette.fg3)
      }
    }
  }
}

/// agentOrder.ts pickDefaultAgent: the configured default unless it is off or its CLI is missing; then the first
/// enabled installed agent
func pickDefaultAgent(_ agents: [AgentInfo], _ preferred: AgentId) -> AgentId {
  guard let own = agents.first(where: { $0.id == preferred }), own.disabled == true || own.available == false else { return preferred }
  let open = launchable(agents)
  return (open.first { $0.available != false } ?? (own.disabled == true ? open.first : nil))?.id ?? preferred
}

/// The agent's mark and name, and "Off" while it is switched off
struct AgentRow: View {
  let agent: AgentInfo

  var body: some View {
    LabeledContent {
      if agent.disabled == true { Text(web: t("mobile.settings.agentOff"), paragraph: false) }
    } label: {
      Label {
        Text(verbatim: agent.name).lineLimit(1)
      } icon: {
        VendorMark(brand: agent.id, name: agent.name, size: Metrics.rowMark, fit: true).foregroundStyle(Palette.fg2)
      }
    }
  }
}

/// One agent: on or off for new sessions, Plan-mode approval, and the models its menus show
struct AgentSettingsPage: View {
  @Environment(AppModel.self) private var model
  let id: AgentId

  var body: some View {
    let agent = model.agent(id)
    let settings = model.settings ?? .defaults
    let enabled = model.settingsAgents.filter { $0.disabled != true }
    Form {
      Section {
        Toggle(isOn: Binding(get: { agent.disabled != true }, set: { model.setAgentEnabled(id, $0) })) {
          Text(web: t("settings.agent.enable"), paragraph: false)
        }
        .tint(.green)
        // The last agent on stays on: a new session needs one
        .disabled(enabled.map(\.id) == [id])
      }
      Section {
        Toggle(isOn: Binding(get: { settings.planAutoApprove.contains(id) }, set: { model.setPlanAutoApprove(id, $0) })) {
          Text(web: t("settings.planAutoApprove"), paragraph: false)
        }
        .tint(.green)
      } header: {
        Text(web: t("settings.agent.permissions"))
      } footer: {
        Text(web: t("settings.planAutoApprove.desc", ["agent": agent.name]))
      }
      Section {
        NavigationLink(value: SettingsRoute.models(id)) {
          LabeledContent(t("settings.models.selection")) {
            if let count = ModelSettings.count(model.agentControls[id], hidden: settings.hiddenOptions[id] ?? [:]) {
              Text(verbatim: "\(count.shown)")
            }
          }
        }
      } footer: {
        Text(web: t("settings.models.desc"))
      }
    }
    .formPage()
    .navigationTitle(agent.name)
    .navigationBarTitleDisplayModeInline()
    .onAppear { model.loadControls(id) }
  }
}

/// The model menus' families with a switch each (AgentPage.tsx ModelsSection): a search, the all-models switch, then
/// one group per control (official / custom models apart where the adapter says which is which)
struct ModelsSettingsPage: View {
  @Environment(AppModel.self) private var model
  let id: AgentId
  @State private var query = ""

  var body: some View {
    let agent = model.agent(id)
    let hidden = model.settings?.hiddenOptions[id] ?? [:]
    let catalogs = ModelSettings.catalogs(model.agentControls[id])
    Form {
      if let catalogs {
        if catalogs.isEmpty {
          Section { Text(web: t("settings.models.none", ["agent": agent.name])).foregroundStyle(Palette.fg3) }
        } else {
          let total = catalogs.reduce(0) { $0 + $1.families.count }
          let shown = catalogs.reduce(0) { n, c in n + c.families.filter { !ModelCatalog.hidden($0, in: hidden[c.control.id] ?? []) }.count }
          if query.isEmpty {
            Section {
              Toggle(isOn: Binding(get: { shown == total }, set: { on in
                for c in catalogs {
                  model.setHidden(id, control: c.control.id, ModelCatalog.setAllVisible(c.families, hidden: hidden[c.control.id] ?? [], visible: on))
                }
              })) {
                Text(web: t("settings.models.all"), paragraph: false)
              }
              .tint(.green)
            } footer: {
              Text(verbatim: t("settings.models.enabled", ["count": shown, "total": total]))
            }
          }
          let groups = ModelSettings.groups(catalogs, query: query)
          ForEach(groups, id: \.key) { g in
            Section {
              ForEach(g.rows, id: \.key) { f in
                let off = hidden[g.control.id] ?? []
                Toggle(isOn: Binding(get: { !ModelCatalog.hidden(f, in: off) }, set: { on in
                  model.setHidden(id, control: g.control.id,
                                  ModelCatalog.setVisible(g.control.options, hidden: model.settings?.hiddenOptions[id]?[g.control.id] ?? [],
                                                          key: f.key, show: on))
                })) {
                  SettingLabel(title: f.name, hint: ModelSettings.summary(f) ?? f.source)
                }
                .tint(.green)
              }
            } header: {
              // A lone group is the page itself, whose title already names it
              if groups.count > 1 { Text(verbatim: g.title) }
            } footer: {
              if !g.sources.isEmpty { Text(verbatim: g.sources.joined(separator: t("common.metaSep"))) }
            }
          }
          if !query.isEmpty && groups.isEmpty {
            Section { Text(web: t("settings.models.noResults")).foregroundStyle(Palette.fg3) }
          }
        }
      } else {
        Section { Text(web: t("settings.loading")).foregroundStyle(Palette.fg3) }
      }
    }
    .formPage()
    .searchable(text: $query, prompt: Text(web: t("settings.models.search")))
    .navigationTitle(t("settings.models.selection"))
    .navigationBarTitleDisplayModeInline()
    .toolbar {
      // The desktop's refresh: the engine asks the agent again (a fresh probe), as the empty state says
      ToolbarItem(placement: .primaryAction) {
        Button { model.loadControls(id, fresh: true) } label: { Image(systemName: "arrow.clockwise") }
          .accessibilityLabel(Text(web: t("common.refresh")))
      }
    }
    .onAppear { model.loadControls(id) }
  }
}

/// The models page's data (AgentPage.tsx ModelsSection), apart from the views so it can be tested
enum ModelSettings {
  struct Catalog { var control: ConfigControl; var families: [ModelFamily] }
  struct Group { var control: ConfigControl; var key: String; var title: String; var rows: [ModelFamily]; var sources: [String] }

  /// The controls whose options are models (no reasoning levels, no switches, not the built-in agent's sidekick pick,
  /// its levels and its Fast), their families newest first; nil before the engine answered
  static func catalogs(_ controls: [ConfigControl]?) -> [Catalog]? {
    let sidekick = [ComposerControls.sidekickCategory, ComposerControls.sidekickEffortCategory, ComposerControls.sidekickFastCategory]
    return controls?.filter { !ComposerControls.isReasoning($0) && $0.type != "boolean" && !sidekick.contains($0.category ?? "") }
      .map { Catalog(control: $0, families: ModelCatalog.prioritize(ModelCatalog.group($0.options))) }
  }

  static func count(_ controls: [ConfigControl]?, hidden: [String: [String]]) -> (shown: Int, total: Int)? {
    guard let catalogs = catalogs(controls), !catalogs.isEmpty else { return nil }
    let total = catalogs.reduce(0) { $0 + $1.families.count }
    let shown = catalogs.reduce(0) { n, c in n + c.families.filter { !ModelCatalog.hidden($0, in: hidden[c.control.id] ?? []) }.count }
    return (shown, total)
  }

  /// One group per control, the model control split into official / custom / other where the adapter classified its
  /// sources
  static func groups(_ catalogs: [Catalog], query: String) -> [Group] {
    catalogs.flatMap { c -> [Group] in
      let families = ModelCatalog.filter(c.families, query: query)
      guard !families.isEmpty else { return [] }
      let title = c.control.category == "model" ? t("settings.models.selection")
        : c.control.category == "collaboration_mode" ? t("settings.models.collaboration") : c.control.name
      let split = c.control.category == "model" && families.contains { $0.sourceKind != nil }
      let kinds: [(key: String, title: String, kind: String??)] = split
        ? [("official", t("settings.models.official"), .some("official")), ("custom", t("settings.models.custom"), .some("custom")),
           ("other", title, .some(nil))]
        : [(c.control.id, title, nil)]
      return kinds.compactMap { g in
        let rows = families.filter { f in g.kind.map { f.sourceKind == $0 } ?? true }
        guard !rows.isEmpty else { return nil }
        var seen = Set<String>()
        let sources = rows.compactMap(\.source).filter { seen.insert($0).inserted }
        return Group(control: c.control, key: "\(c.control.id):\(g.key)", title: g.title, rows: rows, sources: sources)
      }
    }
  }

  /// What a family spans, its row's second line: its effort levels, Fast, 1M; a single variant names itself
  static func summary(_ f: ModelFamily) -> String? {
    if f.variants.count == 1, let v = f.variants.first {
      if v.name == f.name { return nil }
      if v.lead != nil { return ModelCatalog.fusionLabel(v) }
      let parts = [v.effort, v.fast ? "Fast" : "", v.long ? "1M" : ""].filter { !$0.isEmpty }
      return parts.isEmpty ? t("composer.standard") : parts.joined(separator: " ")
    }
    let fusion = f.fusion.map { t("composer.fusionSummary", ["leads": $0.leads.count, "sidekicks": $0.sidekicks.count]) }
    let levels = f.efforts.filter { !$0.isEmpty }
    let parts = [fusion, levels.isEmpty ? nil : "\(t("settings.providers.model.thinking")) \(levels.joined(separator: " / "))",
                 f.hasFast ? "Fast" : nil, f.hasLong ? t("settings.providers.model.ctx", ["n": "1M"]) : nil].compactMap { $0 }
    return parts.isEmpty ? nil : parts.joined(separator: t("common.metaSep"))
  }
}
