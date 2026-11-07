package cc.hotic.acpira.store

import cc.hotic.acpira.model.AgentId
import cc.hotic.acpira.model.AgentInfo
import cc.hotic.acpira.model.SettingsView
import cc.hotic.acpira.model.WebviewMsg
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive

// apps/ios Store/AppModel+Settings.swift: the engine's settings the phone edits (settings.ts SettingsView), stored by
// the engine on the computer, shared with its desktop app and every other window, pushed back as `settings` (and
// `agents` when the order or the switches change). The phone applies them at once so a switch does not wait for the
// round trip; the demo keeps them here.

/** One setting changed: [next] shown at once, [key] / [value] (its SettingKey and wire value) sent to the engine */
private fun AppModel.update(next: SettingsView, key: String, value: JsonElement) {
  settings = next
  // The engine re-sorts and flags its agents itself (arrangeAgents) and pushes them back; they are arranged here too,
  // so an agent's switch (which reads the agent's flag) does not spring back while the push is on its way
  if (key == "agentOrder" || key == "disabledAgents") agents = arrangeAgents(agents, next)
  post(WebviewMsg.SetSetting(key, value))
}

private fun ids(list: List<String>) = JsonArray(list.map(::JsonPrimitive))

/** agentOrder.ts pickDefaultAgent: the configured default unless it is off or its CLI is missing; then the first
 *  enabled installed agent */
fun pickDefaultAgent(agents: List<AgentInfo>, preferred: AgentId): AgentId {
  val own = agents.firstOrNull { it.id == preferred }
  if (own == null || (own.disabled != true && own.available != false)) return preferred
  val open = agents.filter { it.disabled != true }
  return (open.firstOrNull { it.available != false } ?: if (own.disabled == true) open.firstOrNull() else null)?.id ?: preferred
}

/** Switches an agent on or off for new sessions; the last one on stays on (the desktop greys its switch out) */
fun AppModel.setAgentEnabled(id: AgentId, on: Boolean) {
  if (!on && agents.filter { it.disabled != true }.map { it.id } == listOf(id)) return
  val rest = settings.disabledAgents.filter { it != id }
  val next = if (on) rest else rest + id
  update(settings.copy(disabledAgents = next), "disabledAgents", ids(next))
}

/** The listed agent at [from] moved to [to] (SettingsShell `onReorder`): ids the list does not show keep their place
 *  after them */
fun AppModel.moveAgent(from: Int, to: Int) {
  val list = agents.map { it.id }.toMutableList()
  if (from !in list.indices || to !in list.indices || from == to) return
  list.add(to, list.removeAt(from))
  val next = list + settings.agentOrder.filter { it !in list }
  update(settings.copy(agentOrder = next), "agentOrder", ids(next))
}

fun AppModel.setPlanAutoApprove(id: AgentId, on: Boolean) {
  val rest = settings.planAutoApprove.filter { it != id }
  val next = if (on) rest + id else rest
  update(settings.copy(planAutoApprove = next), "planAutoApprove", ids(next))
}

/** An agent's hidden families for one control (AgentPage `save`): empty lists and agents drop out of the map */
fun AppModel.setHidden(agent: AgentId, control: String, keys: List<String>) {
  val forAgent = settings.hiddenOptions[agent].orEmpty().toMutableMap()
  if (keys.isEmpty()) forAgent.remove(control) else forAgent[control] = keys
  val all = settings.hiddenOptions.toMutableMap()
  if (forAgent.isEmpty()) all.remove(agent) else all[agent] = forAgent
  val wire = JsonObject(all.mapValues { (_, m) -> JsonObject(m.mapValues { ids(it.value) }) })
  update(settings.copy(hiddenOptions = all), "hiddenOptions", wire)
}

fun AppModel.setAccountSwitch(value: String) =
  update(settings.copy(accountSwitch = value), "accountSwitch", JsonPrimitive(value))

fun AppModel.setSteerQueued(on: Boolean) = update(settings.copy(steerQueued = on), "steerQueued", JsonPrimitive(on))

fun AppModel.setAutoCompact(on: Boolean) = update(settings.copy(autoCompact = on), "autoCompact", JsonPrimitive(on))

fun AppModel.setCompactAt(tokens: Int) = update(settings.copy(compactAtTokens = tokens), "compactAtTokens", JsonPrimitive(tokens))

/** The desktop's refresh: the engine asks the agent again (a fresh probe) */
fun AppModel.refreshControls(agent: AgentId) {
  if (!post(WebviewMsg.Controls(agent, fresh = true))) loadControls(agent)
}

/** agentOrder.ts arrangeAgents: the saved order first, the rest in registry order after them;
 *  switched-off agents are flagged, never dropped */
fun arrangeAgents(list: List<AgentInfo>, settings: SettingsView): List<AgentInfo> {
  val rank = HashMap<String, Int>()
  settings.agentOrder.forEachIndexed { i, id -> rank.putIfAbsent(id, i) }
  val off = settings.disabledAgents.toSet()
  return list.mapIndexed { i, a ->
    Triple(a.copy(disabled = if (a.id in off) true else null), rank[a.id] ?: settings.agentOrder.size, i)
  }.sortedWith(compareBy({ it.second }, { it.third })).map { it.first }
}

/** 0 first (settings.ts COMPACT_AT_MODEL: each session's own model window), then MIN_COMPACT_AT_TOKENS and up, the
 *  current value kept in its place (a count typed on the desktop); the top preset matches the 1M windows */
fun compactPresets(current: Int): List<Int> =
  (listOf(0, 100_000, 200_000, 300_000, 400_000, 600_000, 800_000, 1_000_000) + current).distinct().sorted()
