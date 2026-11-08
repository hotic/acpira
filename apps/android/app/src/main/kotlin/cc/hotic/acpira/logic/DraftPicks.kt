package cc.hotic.acpira.logic

import cc.hotic.acpira.model.SessionControls

/** What a new session's page picked, to be set on the session the engine made: a mode, then config values in order */
data class PickReplay(val mode: String?, val config: List<Pair<String, String>>)

/**
 * The new-session page's [picked] controls over what the engine started the session with ([made]) (apps/ios
 * AppModel+Engine.swift `apply`). A mode goes only where it differs and the session offers it. A changed model replays
 * every picked value after it, equal ones included: the model switch can reset an equal effort or bring a control that
 * was absent at startup, and the engine checks each value against the new model. Otherwise only the values that differ
 * from a control the session has
 */
fun replayPicks(picked: SessionControls, made: SessionControls): PickReplay {
  val mode = picked.modeId?.takeIf { m -> m != made.modeId && made.modes.any { it.id == m } }
  val modelChanged = picked.options.any { option ->
    isModelControl(option) && option.value != made.options.firstOrNull { it.id == option.id }?.value
  }
  val config = picked.options.mapNotNull { option ->
    val value = option.value ?: return@mapNotNull null
    val now = made.options.firstOrNull { it.id == option.id }
    if (modelChanged || (now != null && now.value != value)) option.id to value else null
  }
  return PickReplay(mode, config)
}
