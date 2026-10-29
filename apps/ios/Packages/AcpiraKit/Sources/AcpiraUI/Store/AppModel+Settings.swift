import AcpiraModels
import Foundation
import SwiftUI

// The engine's settings the phone edits (settings.ts SettingsView): stored by the engine on the computer, shared with
// its desktop app and every other window, pushed back as `settings` (and `agents` when the order or the switches
// change). The phone sets them at once so a switch does not wait for the round trip; the demo keeps them here.

extension SettingsView {
  /// The engine's defaults: every field's fallback (decoding `{}` cannot fail, every field has one)
  static var defaults: SettingsView {
    try! JSONDecoder().decode(SettingsView.self, from: Data("{}".utf8))
  }
}

extension AppModel {
  /// One setting changed; `name` is its SettingKey
  func setSetting<T: Encodable>(_ key: WritableKeyPath<SettingsView, T>, _ name: String, _ value: T) {
    settings?[keyPath: key] = value
    // The engine re-sorts and flags its agents itself (arrangeAgents) and pushes them back; they are arranged here too,
    // so an agent's switch (which reads the agent's flag) does not spring back while the push is on its way
    if name == "agentOrder" || name == "disabledAgents", let settings { agents = arrangeAgents(agents, settings) }
    guard let data = try? JSONEncoder().encode(value), let json = try? JSONDecoder().decode(JSONValue.self, from: data) else { return }
    _ = post(.setSetting(key: name, value: json))
  }

  /// The agents the settings list and reorder: every one but the external entries (ChatGPT's bridge)
  var settingsAgents: [AgentInfo] { agents.filter { $0.external != true } }

  /// Switches an agent on or off for new sessions; the last one on stays on (the desktop greys its switch out)
  func setAgentEnabled(_ id: AgentId, _ on: Bool) {
    guard let settings else { return }
    if !on && settingsAgents.filter({ $0.disabled != true }).map(\.id) == [id] { return }
    let next = on ? settings.disabledAgents.filter { $0 != id } : settings.disabledAgents.filter { $0 != id } + [id]
    setSetting(\.disabledAgents, "disabledAgents", next)
  }

  /// The listed agents' new order (SettingsShell `onReorder`): ids the list does not show keep their place after them
  func moveAgents(from: IndexSet, to: Int) {
    guard let settings else { return }
    var ids = settingsAgents.map(\.id)
    ids.move(fromOffsets: from, toOffset: to)
    setSetting(\.agentOrder, "agentOrder", ids + settings.agentOrder.filter { !ids.contains($0) })
  }

  func setPlanAutoApprove(_ id: AgentId, _ on: Bool) {
    guard let settings else { return }
    let rest = settings.planAutoApprove.filter { $0 != id }
    setSetting(\.planAutoApprove, "planAutoApprove", on ? rest + [id] : rest)
  }

  /// An agent's hidden families for one control (AgentPage `save`): empty lists and agents drop out of the map
  func setHidden(_ agent: AgentId, control: String, _ keys: [String]) {
    guard let settings else { return }
    var forAgent = settings.hiddenOptions[agent] ?? [:]
    forAgent[control] = keys.isEmpty ? nil : keys
    var all = settings.hiddenOptions
    all[agent] = forAgent.isEmpty ? nil : forAgent
    setSetting(\.hiddenOptions, "hiddenOptions", all)
  }

  /// Asks for an agent's controls (the engine answers `controls` from its last session or a probe); the demo reads the
  /// fixture sessions'
  func loadControls(_ agent: AgentId, fresh: Bool = false) {
    if post(.controls(agent: agent, fresh: fresh ? true : nil)) { return }
    if agentControls[agent] == nil {
      agentControls[agent] = views.values.first { $0.agent == agent && !$0.controls.options.isEmpty }?.controls.options ?? []
    }
  }
}

/// agentOrder.ts arrangeAgents: the saved order first, the rest in registry order after them, external entries last;
/// switched-off agents are flagged, never dropped
func arrangeAgents(_ list: [AgentInfo], _ settings: SettingsView) -> [AgentInfo] {
  let rank = Dictionary(settings.agentOrder.enumerated().map { ($1, $0) }, uniquingKeysWith: { a, _ in a })
  let off = Set(settings.disabledAgents)
  return list.enumerated()
    .map { i, a -> (AgentInfo, [Int]) in
      var a = a
      if a.external != true { a.disabled = off.contains(a.id) ? true : nil }
      return (a, [a.external == true ? 1 : 0, rank[a.id] ?? settings.agentOrder.count, i])
    }
    .sorted { $0.1.lexicographicallyPrecedes($1.1) }
    .map(\.0)
}
