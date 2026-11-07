package cc.hotic.acpira.model

import kotlinx.serialization.KSerializer
import kotlinx.serialization.SerializationException
import kotlinx.serialization.builtins.ListSerializer
import kotlinx.serialization.descriptors.PrimitiveKind
import kotlinx.serialization.descriptors.PrimitiveSerialDescriptor
import kotlinx.serialization.descriptors.SerialDescriptor
import kotlinx.serialization.encoding.Decoder
import kotlinx.serialization.encoding.Encoder
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonDecoder
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonEncoder
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive

// The engine contract the app speaks. The source of truth is TypeScript (packages/shared/sidecar.ts, protocol.ts,
// transcript.ts), mirrored in Rust (rust/crates/acpira-shared) and Swift (apps/ios AcpiraModels); this module mirrors
// the subset the Android app renders. Decoding stays tolerant: the engine on the computer updates faster than a store
// ships the app, so an unknown kind decodes into a placeholder and a missing field into its default instead of
// failing the whole view.

/** The one Json configuration every wire read and write goes through */
val WireJson: Json = Json {
  ignoreUnknownKeys = true
  // A null where the contract has a default (an engine of another release) reads as the default
  coerceInputValues = true
  // Absent optional fields stay absent on the wire, as the TypeScript side writes them
  explicitNulls = false
  encodeDefaults = false
}

/**
 * A string enum that keeps unknown values instead of throwing. Entries are named exactly as their wire literal
 * (`in_progress`), so `name` is the wire form unless [wire] says otherwise
 */
abstract class TolerantEnumSerializer<E : Enum<E>>(
  serialName: String,
  private val entries: List<E>,
  private val fallback: E,
  private val wire: (E) -> String = { it.name },
) : KSerializer<E> {
  override val descriptor: SerialDescriptor = PrimitiveSerialDescriptor(serialName, PrimitiveKind.STRING)
  override fun deserialize(decoder: Decoder): E {
    val raw = decoder.decodeString()
    return entries.firstOrNull { wire(it) == raw } ?: fallback
  }
  override fun serialize(encoder: Encoder, value: E) = encoder.encodeString(wire(value))
}

/**
 * A union tagged by one string field (`type`, `role`, `kind`): [decode] picks the case, and anything it does not know or
 * cannot read becomes [unknown], so one odd block never fails the view around it
 */
abstract class TaggedSerializer<T : Any>(serialName: String, private val tag: String) : KSerializer<T> {
  override val descriptor: SerialDescriptor = PrimitiveSerialDescriptor(serialName, PrimitiveKind.STRING)

  protected abstract fun decode(json: Json, tag: String, obj: JsonObject): T?
  protected abstract fun unknown(tag: String): T
  protected abstract fun encode(json: Json, value: T): JsonObject

  override fun deserialize(decoder: Decoder): T {
    val input = decoder as? JsonDecoder ?: throw SerializationException("$tag-tagged values are JSON only")
    val obj = input.decodeJsonElement().jsonObject
    val value = (obj[tag] as? JsonPrimitive)?.contentOrNull ?: return unknown("")
    return try {
      decode(input.json, value, obj) ?: unknown(value)
    } catch (_: SerializationException) {
      unknown(value)
    } catch (_: IllegalArgumentException) {
      unknown(value)
    }
  }

  override fun serialize(encoder: Encoder, value: T) {
    val output = encoder as? JsonEncoder ?: throw SerializationException("$tag-tagged values are JSON only")
    output.encodeJsonElement(encode(output.json, value))
  }
}

/** A list whose unreadable elements are dropped instead of failing the value around it; anything but an array reads as empty */
open class TolerantListSerializer<T>(private val element: KSerializer<T>) : KSerializer<List<T>> {
  private val strict = ListSerializer(element)
  override val descriptor: SerialDescriptor = strict.descriptor
  override fun deserialize(decoder: Decoder): List<T> {
    val input = decoder as? JsonDecoder ?: return strict.deserialize(decoder)
    val array = input.decodeJsonElement() as? JsonArray ?: return emptyList()
    return array.mapNotNull { item ->
      try {
        input.json.decodeFromJsonElement(element, item)
      } catch (_: SerializationException) {
        null
      } catch (_: IllegalArgumentException) {
        null
      }
    }
  }
  override fun serialize(encoder: Encoder, value: List<T>) = strict.serialize(encoder, value)
}

/** [obj] with [tag] set first, the way the TypeScript side writes a tagged value */
fun tagged(tag: String, value: String, obj: JsonElement): JsonObject =
  JsonObject(linkedMapOf<String, JsonElement>(tag to JsonPrimitive(value)) + obj.jsonObject.filterKeys { it != tag })

/** A string field of a raw object, or null */
fun JsonObject.string(key: String): String? = (this[key] as? JsonPrimitive)?.takeIf { it.isString }?.content

/** An int field of a raw object, or null */
fun JsonObject.int(key: String): Int? = (this[key] as? JsonPrimitive)?.contentOrNull?.toIntOrNull()

/** A boolean field of a raw object, or null */
fun JsonObject.bool(key: String): Boolean? = (this[key] as? JsonPrimitive)?.booleanOrNull

/** A number field of a raw object, or null */
fun JsonObject.double(key: String): Double? = (this[key] as? JsonPrimitive)?.takeIf { !it.isString }?.contentOrNull?.toDoubleOrNull()

/** A primitive's text whatever its kind, for logs */
fun JsonElement.preview(limit: Int = 120): String = toString().take(limit)

internal fun JsonElement.stringOrNull(): String? = (this as? JsonPrimitive)?.takeIf { it.isString }?.content

internal fun JsonElement.primitiveContent(): String? = runCatching { jsonPrimitive.contentOrNull }.getOrNull()
