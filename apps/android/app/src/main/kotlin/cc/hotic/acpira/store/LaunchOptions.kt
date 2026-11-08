package cc.hotic.acpira.store

import android.content.Intent

/**
 * What the app opens on, from the launch intent's string extras (`adb shell am start -n … --es screen chat`): the iOS
 * app's LaunchOptions under the same names, so scripts drive both apps alike (.agents/skills/touch-sim)
 */
data class LaunchOptions(
  /** `list | chat | new | settings | changes | file`: the screen on launch (`changes` / `file` over the fixture's chat) */
  val screen: String = "list",
  /** The demo session a chat opens on (a fixture name: showcase, running, streaming …) */
  val fixture: String = "showcase",
  /** `light | dark`; absent: the system's */
  val theme: String? = null,
  /** A dictionary id (`zh-CN`); absent: the device language */
  val locale: String? = null,
  /** `PORT:TOKEN` of the development engine `acpira --ws` (apps/ios/scripts/engine.sh), reached through adb reverse */
  val engine: String? = null,
  /** With an engine: that session's chat opens on launch */
  val session: String? = null,
  /** With an engine: a session of [agent] (the default one when absent) starts on launch, [prompt] its first prompt */
  val start: Boolean = false,
  val agent: String? = null,
  /** The chat sends this on launch */
  val prompt: String? = null,
  /** `none`: entrances and spinners rest (screenshots) */
  val motion: Boolean = true,
  /** The chat's composer opens holding this draft (screenshots of the card) */
  val draft: String? = null,
  /** A composer picker open on launch, as the iOS app's `-popup`: `model`, `model.2` (its second bean's panel), `attach` */
  val popup: String? = null,
  /**
   * Debug builds: `ID@HOST:PORT`, a computer the add-computer sheet lists as nearby. The emulator cannot browse or
   * reach the computer's LAN, only its loopback as 10.0.2.2, so scripts/e2e-paired.sh pairs through this and the code
   */
  val nearby: String? = null,
  /**
   * `play | intro | pair`: the first-run flow on launch whatever has been paired, from its start or on that step with
   * the intro already drawn (screenshots)
   */
  val welcome: String? = null,
  /** `ID | tree`: that child's inspector, or the session's subagent graph, pushed over the launch chat */
  val subagent: String? = null,
  /** With an engine: that project folder's terminal opens on launch */
  val terminal: String? = null,
  /** `hud.fps` / `hud.hitches` = `YES | NO`: Settings → Developer's overlay switches for this launch, not stored
   *  (the iOS app's `-acpira.hud.fps YES`) */
  val hudFps: Boolean? = null,
  val hudHitches: Boolean? = null,
  /** The launch asked for the demo's fixtures (`screen`, `fixture` or `demo`): no pairing, the synthetic sessions */
  val demo: Boolean = false,
) {
  companion object {
    fun from(intent: Intent?): LaunchOptions {
      fun s(key: String) = intent?.getStringExtra(key)?.takeIf { it.isNotEmpty() }
      return LaunchOptions(
        screen = s("screen") ?: if (s("session") != null || s("start") != null) "chat" else "list",
        fixture = s("fixture") ?: "showcase",
        theme = s("theme"),
        locale = s("locale"),
        engine = s("engine"),
        session = s("session"),
        start = s("start") != null,
        agent = s("agent"),
        prompt = s("prompt"),
        motion = s("motion") != "none",
        draft = s("draft"),
        popup = s("popup"),
        nearby = s("nearby"),
        welcome = s("welcome"),
        subagent = s("subagent"),
        terminal = s("terminal"),
        hudFps = flag(s("hud.fps")),
        hudHitches = flag(s("hud.hitches")),
        demo = s("screen") != null || s("fixture") != null || s("demo") != null || s("popup") != null || s("draft") != null,
      )
    }
  }
}

/** A launch switch's value: `YES` / `true` / `1` on, `NO` / `false` / `0` off, anything else unset */
internal fun flag(value: String?): Boolean? = when (value?.lowercase()) {
  "yes", "true", "1" -> true
  "no", "false", "0" -> false
  else -> null
}

/** Where the development engine listens: `PORT:TOKEN`, as apps/ios/scripts/engine.sh prints it */
data class EngineAddress(val port: Int, val token: String) {
  companion object {
    fun parse(argument: String?): EngineAddress? {
      val parts = argument?.split(":", limit = 2) ?: return null
      if (parts.size != 2 || parts[1].isEmpty()) return null
      return parts[0].toIntOrNull()?.let { EngineAddress(it, parts[1]) }
    }
  }
}
