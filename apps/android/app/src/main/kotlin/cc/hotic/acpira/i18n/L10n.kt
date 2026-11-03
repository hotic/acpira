package cc.hotic.acpira.i18n

import android.content.Context
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.jsonPrimitive
import java.util.Locale

/**
 * The webview's dictionaries (the iOS app's exported copies, synced into assets/i18n) and its `t()`: `{name}`
 * placeholders, a count picks the plural variant, a missing key falls back to English, then to the key itself
 */
object L10n {
  val locales = listOf("en", "zh-CN", "zh-TW", "ja", "ko", "de", "fr", "es", "ru")

  private var table: Map<String, String> = emptyMap()
  private var english: Map<String, String> = emptyMap()
  var locale: String = "en"
    private set

  /** Loads the tables once; [id] overrides the device language (the `locale` launch extra) */
  fun setup(context: Context, id: String?) {
    english = load(context, "en")
    locale = id?.takeIf { it in locales } ?: preferred()
    table = if (locale == "en") english else load(context, locale)
  }

  /** The device's first language mapped onto the shipped tables */
  private fun preferred(): String {
    val l = Locale.getDefault()
    if (l.language == "zh") return if (l.script == "Hant" || l.country in setOf("TW", "HK", "MO")) "zh-TW" else "zh-CN"
    return l.language.takeIf { it in locales } ?: "en"
  }

  private fun load(context: Context, id: String): Map<String, String> = runCatching {
    val text = context.assets.open("i18n/$id.json").bufferedReader().use { it.readText() }
    (Json.parseToJsonElement(text) as JsonObject).mapValues { it.value.jsonPrimitive.content }
  }.getOrDefault(emptyMap())

  /** i18n/index.ts pluralCategory: CLDR cardinal rules for the shipped locales; null is the plain key */
  private fun pluralCategory(n: Int): String? {
    if (n < 0) return null
    return when (locale) {
      "ru" -> {
        val d = n % 10; val h = n % 100
        if (d == 1 && h != 11) "one" else if (d in 2..4 && h !in 12..14) "few" else "many"
      }
      "fr" -> if (n < 2) "one" else null
      "en", "es", "de" -> if (n == 1) "one" else null
      else -> null
    }
  }

  fun t(key: String, params: Map<String, Any> = emptyMap()): String {
    val count = (params["n"] ?: params["count"])?.toString()?.toIntOrNull()
    val variant = count?.let { pluralCategory(it) }?.let { table["$key#$it"] }
    var text = variant ?: table[key] ?: english[key] ?: key
    for ((name, value) in params) text = text.replace("{$name}", value.toString())
    return text
  }
}

/** Shorthand matching the webview's `t(key, params)` */
fun t(key: String, vararg params: Pair<String, Any>): String = L10n.t(key, params.toMap())
