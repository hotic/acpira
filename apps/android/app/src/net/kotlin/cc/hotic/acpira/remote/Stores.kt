package cc.hotic.acpira.remote

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
import cc.hotic.acpira.sync.PairedComputer
import kotlinx.serialization.builtins.ListSerializer
import kotlinx.serialization.json.Json
import java.io.File
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/**
 * Where this device's identity key lives: 32 bytes whose public half is the EndpointId every paired computer pinned.
 * Losing it means pairing again; copying it to another device would make two devices one identity
 */
interface IdentityStore {
  fun load(): ByteArray?
  fun save(key: ByteArray)
}

/**
 * The key sealed with an AES key that never leaves the Android Keystore (apps/ios KeychainIdentityStore keeps it in the
 * Keychain, this device only). The sealed bytes sit in the app's private preferences; the manifest turns backups off,
 * so a restored or second device cannot share the identity
 */
class KeystoreIdentityStore(context: Context) : IdentityStore {
  private val prefs = context.getSharedPreferences("remote", Context.MODE_PRIVATE)

  override fun load(): ByteArray? {
    val sealed = prefs.getString(KEY, null) ?: return null
    val bytes = Base64.decode(sealed, Base64.NO_WRAP)
    if (bytes.size <= IV) return null
    val cipher = Cipher.getInstance(TRANSFORM)
    cipher.init(Cipher.DECRYPT_MODE, wrapKey(), GCMParameterSpec(TAG_BITS, bytes, 0, IV))
    return cipher.doFinal(bytes, IV, bytes.size - IV)
  }

  override fun save(key: ByteArray) {
    val cipher = Cipher.getInstance(TRANSFORM)
    cipher.init(Cipher.ENCRYPT_MODE, wrapKey())
    val sealed = cipher.iv + cipher.doFinal(key)
    prefs.edit().putString(KEY, Base64.encodeToString(sealed, Base64.NO_WRAP)).commit()
  }

  private fun wrapKey(): SecretKey {
    val store = KeyStore.getInstance(PROVIDER).apply { load(null) }
    (store.getKey(ALIAS, null) as? SecretKey)?.let { return it }
    val generator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, PROVIDER)
    generator.init(
      KeyGenParameterSpec.Builder(ALIAS, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
        .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
        .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
        .setKeySize(256)
        .build(),
    )
    return generator.generateKey()
  }

  private companion object {
    const val PROVIDER = "AndroidKeyStore"
    const val ALIAS = "acpira.remote.identity"
    const val KEY = "identity"
    const val TRANSFORM = "AES/GCM/NoPadding"
    const val IV = 12
    const val TAG_BITS = 128
  }
}

/** The paired computers as JSON in the app's files (apps/ios ComputerStore); written tmp + rename */
class ComputerStore(directory: File) {
  private val file = File(directory, "computers.json")

  fun load(): List<PairedComputer> = runCatching {
    json.decodeFromString(ListSerializer(PairedComputer.serializer()), file.readText())
  }.getOrDefault(emptyList())

  fun save(computers: List<PairedComputer>) {
    file.parentFile?.mkdirs()
    val tmp = File(file.parentFile, "${file.name}.tmp")
    tmp.writeText(json.encodeToString(ListSerializer(PairedComputer.serializer()), computers))
    if (!tmp.renameTo(file)) {
      file.delete()
      tmp.renameTo(file)
    }
  }

  private companion object {
    val json = Json { ignoreUnknownKeys = true; prettyPrint = true }
  }
}
