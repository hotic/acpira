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

  /// Switches an agent on or off for new sessions; the last one on stays on (the desktop greys its switch out)
  func setAgentEnabled(_ id: AgentId, _ on: Bool) {
    guard let settings else { return }
    if !on && agents.filter({ $0.disabled != true }).map(\.id) == [id] { return }
    let next = on ? settings.disabledAgents.filter { $0 != id } : settings.disabledAgents.filter { $0 != id } + [id]
    setSetting(\.disabledAgents, "disabledAgents", next)
  }

  /// The listed agents' new order (SettingsShell `onReorder`): ids the list does not show keep their place after them
  func moveAgents(from: IndexSet, to: Int) {
    guard let settings else { return }
    var ids = agents.map(\.id)
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
  /// fixture sessions'. A folder on an SSH host goes with the ask: the engine answers for where a session there starts
  func loadControls(_ agent: AgentId, fresh: Bool = false, cwd: String = "") {
    let host = hostControlsKey(agent, cwd) == nil ? nil : cwd
    if post(.controls(agent: agent, fresh: fresh ? true : nil, cwd: host)) { return }
    if agentControls[agent] == nil {
      agentControls[agent] = views.values.first { $0.agent == agent && !$0.controls.options.isEmpty }?.controls.options ?? []
    }
  }

  /// Asks for every agent not answered yet, so a switch on the new session page finds the agent's models at once
  /// instead of dropping the model chip for the round trip (hundreds of milliseconds over a paired link)
  func prefetchControls(_ agents: [AgentId], cwd: String = "") {
    for agent in agents where knownControls(agent, cwd: cwd) == nil { loadControls(agent, cwd: cwd) }
  }

  /// The engine's last answer for a new session of `agent` in `cwd`, nil before one came
  func knownControls(_ agent: AgentId, cwd: String = "") -> [ConfigControl]? {
    guard let key = hostControlsKey(agent, cwd) else { return agentControls[agent] }
    return hostControls[key]?.options
  }

  /// An agent the engine knows no models for yet (never run on that computer, no probe) gets one fresh probe on this
  /// wire: the desktop starts the CLI with the page and shows its models, the phone only starts it with the first prompt
  func probeUnknownControls(_ agent: AgentId, cwd: String = "") {
    let key = hostControlsKey(agent, cwd) ?? agent
    guard link != nil, knownControls(agent, cwd: cwd)?.isEmpty == true, !probedControls.contains(key) else { return }
    probedControls.insert(key)
    loadControls(agent, fresh: true, cwd: cwd)
  }
}

/// The engine's `controls` answer for a new session in a folder on an SSH host
struct HostControls: Equatable {
  var options: [ConfigControl]
  var shapes: ModelShapes?
  var commands: [SlashCommand]
}

/// Where `hostControls` keeps an agent's answer for a folder on an SSH host (`ssh://<alias>/<path>`); nil for any other
/// folder, whose new sessions start on the computer with its agent-wide lists
func hostControlsKey(_ agent: AgentId, _ cwd: String) -> String? {
  cwd.hasPrefix("ssh://") ? "\(cwd)\n\(agent)" : nil
}

/// agentOrder.ts arrangeAgents: the saved order first, the rest in registry order after them;
/// switched-off agents are flagged, never dropped
func arrangeAgents(_ list: [AgentInfo], _ settings: SettingsView) -> [AgentInfo] {
  let rank = Dictionary(settings.agentOrder.enumerated().map { ($1, $0) }, uniquingKeysWith: { a, _ in a })
  let off = Set(settings.disabledAgents)
  return list.enumerated()
    .map { i, a -> (AgentInfo, [Int]) in
      var a = a
      a.disabled = off.contains(a.id) ? true : nil
      return (a, [rank[a.id] ?? settings.agentOrder.count, i])
    }
    .sorted { $0.1.lexicographicallyPrecedes($1.1) }
    .map(\.0)
}
