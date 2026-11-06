package cc.hotic.acpira.model

import kotlinx.serialization.Serializable
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put

// Mirror of packages/shared/providers.ts (the built-in agent's model sources) and of the localDir folder listing
// (remote.ts RemoteDirEntry) a paired device browses the computer with. providers.json entries carry fields this build
// does not know, and the engine keeps whatever an entry sent back holds, so a source and its models stay raw objects:
// the accessors read them, every edit is a copy with one field replaced.

/** One entry of a folder listing: folders only, dot-folders left out; `git` when it holds a `.git` */
@Serializable
data class DirEntry(val name: String = "", val dir: Boolean = true, val git: Boolean? = null)

/**
 * A folder inside a listed [path], as the computer spells it: the computer may be Windows, so the separator is the
 * one the listed path uses (a backslash only when it has no slash), and a root that already ends in it gets no second
 */
fun childPath(path: String, name: String): String {
  val sep = if (path.contains('\\') && !path.contains('/')) '\\' else '/'
  return if (path.endsWith(sep)) "$path$name" else "$path$sep$name"
}

/** A path's last part, for a title; a root (`/`, `C:\`) names itself */
fun folderName(path: String): String = path.trimEnd('/', '\\').split('/', '\\').lastOrNull { it.isNotEmpty() } ?: path

private fun JsonObject.bool(key: String): Boolean? = (this[key] as? JsonPrimitive)?.booleanOrNull

private fun JsonObject.with(key: String, value: JsonElement): JsonObject = JsonObject(this + (key to value))

/** One model of a source (providers.ts ProviderModel), raw */
data class ProviderModel(val json: JsonObject) {
  val id: String get() = json.string("id").orEmpty()
  val name: String? get() = json.string("name")?.takeIf { it.isNotEmpty() }
  val enabled: Boolean get() = json.bool("enabled") ?: true

  fun enabled(on: Boolean) = ProviderModel(json.with("enabled", JsonPrimitive(on)))

  companion object {
    /** providers.ts newModel: a hand-typed id with the file's defaults */
    fun new(id: String) = ProviderModel(buildJsonObject {
      put("id", id); put("enabled", true); put("input", JsonArray(listOf(JsonPrimitive("text")))); put("thinking", "auto")
      put("efforts", JsonArray(emptyList())); put("sampling", JsonObject(emptyMap())); put("estimated", JsonArray(emptyList()))
    })
  }
}

/** One model source (providers.ts Provider / ProviderView), raw; `hasKey` is the view's, never the file's */
data class Provider(val json: JsonObject) {
  /** Empty on a new entry: the engine makes one from the name */
  val id: String get() = json.string("id").orEmpty()
  val name: String get() = json.string("name").orEmpty()
  val preset: String get() = json.string("preset") ?: "custom"
  /** An empty format reads as openai-chat (ApiFormat::parse) */
  val format: String get() = json.string("format")?.takeIf { it.isNotEmpty() } ?: "openai-chat"
  val baseUrl: String get() = json.string("baseUrl").orEmpty()
  val fullUrl: Boolean get() = json.bool("fullUrl") ?: false
  val enabled: Boolean get() = json.bool("enabled") ?: true
  val hasKey: Boolean get() = json.bool("hasKey") ?: false
  val models: List<ProviderModel> get() = (json["models"] as? JsonArray).orEmpty().mapNotNull { (it as? JsonObject)?.let(::ProviderModel) }
  /** A server found on the computer (Ollama, LM Studio): it takes no key */
  val local: Boolean get() = preset == "ollama" || preset == "lmstudio"
  val title: String get() = name.ifEmpty { id }

  fun with(key: String, value: JsonElement) = Provider(json.with(key, value))
  fun with(key: String, value: String) = with(key, JsonPrimitive(value))
  fun with(key: String, value: Boolean) = with(key, JsonPrimitive(value))
  fun withModels(models: List<ProviderModel>) = with("models", JsonArray(models.map { it.json }))

  /** What goes back to the engine: the entry as shown, less `hasKey` */
  fun forWire(): JsonObject = JsonObject(json - "hasKey")

  companion object {
    fun new(name: String, format: String, baseUrl: String, fullUrl: Boolean, preset: String = "custom", models: List<ProviderModel> = emptyList()) =
      Provider(buildJsonObject {
        put("id", ""); put("name", name); put("preset", preset); put("format", format); put("baseUrl", baseUrl)
        put("fullUrl", fullUrl); put("enabled", true); put("models", JsonArray(models.map { it.json }))
      })

    /** ProvidersSection.tsx fromLocal: Ollama and LM Studio serve the OpenAI Chat format at the root the probe found */
    fun fromLocal(s: LocalSource) = new(s.name, "openai-chat", s.baseUrl, fullUrl = false, preset = s.preset, models = s.models)
  }
}

/** The sources as the engine shows them (ProvidersView), the part the phone edits; `error`: providers.json is unreadable */
data class ProvidersView(val providers: List<Provider> = emptyList(), val error: String? = null) {
  companion object {
    fun decode(element: JsonElement?): ProvidersView {
      val obj = element as? JsonObject ?: return ProvidersView()
      val list = (obj["providers"] as? JsonArray).orEmpty().mapNotNull { (it as? JsonObject)?.let(::Provider) }
      return ProvidersView(list, obj.string("error"))
    }
  }
}

/** A model server running on the computer, ready to save as a source */
data class LocalSource(val preset: String, val name: String, val baseUrl: String, val models: List<ProviderModel>)

private fun models(element: JsonElement?): List<ProviderModel> =
  (element as? JsonArray).orEmpty().mapNotNull { (it as? JsonObject)?.let(::ProviderModel) }

/** The answer to a providerProbe (ProbeOutcome); a kind this build does not ask for reads as [Other] */
sealed interface ProbeOutcome {
  data class Models(val models: List<ProviderModel>) : ProbeOutcome
  data class Local(val servers: List<LocalSource>) : ProbeOutcome
  data class Failed(val error: String) : ProbeOutcome
  data class Other(val kind: String) : ProbeOutcome

  companion object {
    val kinds = listOf("models", "local", "failed")

    fun decode(element: JsonElement?): ProbeOutcome {
      val obj = element as? JsonObject ?: return Other("")
      return when (val kind = obj.string("kind").orEmpty()) {
        "models" -> Models(models(obj["models"]))
        "local" -> Local((obj["servers"] as? JsonArray).orEmpty().mapNotNull { s ->
          (s as? JsonObject)?.let { LocalSource(it.string("preset").orEmpty(), it.string("name").orEmpty(), it.string("baseUrl").orEmpty(), models(it["models"])) }
        })
        "failed" -> Failed(obj.string("error").orEmpty())
        else -> Other(kind)
      }
    }
  }
}

/** A change to providers.json (ProviderAction), the two a paired device may send. key: absent keeps the stored one */
sealed interface ProviderAction {
  data class Save(val provider: Provider, val key: String? = null) : ProviderAction
  data class Delete(val id: String) : ProviderAction

  fun toJson(): JsonObject = when (this) {
    is Save -> buildJsonObject { put("kind", "save"); put("provider", provider.forWire()); key?.let { put("key", it) } }
    is Delete -> buildJsonObject { put("kind", "delete"); put("id", id) }
  }

  companion object {
    val kinds = listOf("save", "delete")
  }
}

/** A network question that changes nothing (ProviderProbe). key: typed and not saved yet; absent uses the stored one */
sealed interface ProviderProbe {
  data class Models(val provider: Provider, val key: String? = null) : ProviderProbe
  data object Local : ProviderProbe

  fun toJson(): JsonObject = when (this) {
    is Models -> buildJsonObject { put("kind", "models"); put("provider", provider.forWire()); key?.let { put("key", it) } }
    Local -> buildJsonObject { put("kind", "local") }
  }

  companion object {
    val kinds = listOf("models", "local")
  }
}

/** The request URL a source's settings produce, as the agent builds it (ProvidersSection.tsx endpointOf) */
fun endpointOf(baseUrl: String, format: String, fullUrl: Boolean): String {
  val url = baseUrl.trim()
  if (fullUrl) return url
  val suffix = when (format) {
    "anthropic" -> "messages"
    "openai-responses" -> "responses"
    else -> "chat/completions"
  }
  return "${url.trimEnd('/')}/$suffix"
}

/** A URL already naming the endpoint, so it is used as typed (ProvidersSection.tsx looksFull) */
fun looksFull(url: String): Boolean = Regex("""/(chat/completions|responses|messages)/?$""", RegexOption.IGNORE_CASE).containsMatchIn(url.trim())

/** One server address however it is spelled: trailing slashes and case aside */
fun sameUrl(a: String, b: String): Boolean = a.trim().trimEnd('/').equals(b.trim().trimEnd('/'), ignoreCase = true)
