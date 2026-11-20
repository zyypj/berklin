package dev.zyypj.berklin

import android.content.Context
import android.net.Uri
import android.util.Base64
import java.net.URLEncoder

class Pairing(
    val relayUrl: String,
    val token: String,
    val key: ByteArray,
    val pcName: String,
) {
    val roomId: String by lazy { RelayCrypto.roomId(key) }

    fun connectUrl(deviceId: String): String {
        val separator = if (relayUrl.contains('?')) '&' else '?'
        return buildString {
            append(relayUrl)
            append(separator)
            append("room=").append(encode(roomId))
            append("&role=client")
            append("&dev=").append(encode(deviceId))
            append("&t=").append(encode(token))
        }
    }

    private fun encode(value: String): String = URLEncoder.encode(value, "UTF-8").replace("+", "%20")

    companion object {
        private const val SCHEME = "berklin"
        private const val HOST = "pair"

        fun isPairingLink(uri: Uri?): Boolean = uri != null && uri.scheme == SCHEME && uri.host == HOST

        fun fromLink(link: String?): Pairing? {
            if (link.isNullOrBlank()) return null
            return fromUri(Uri.parse(link.trim()))
        }

        fun fromUri(uri: Uri?): Pairing? {
            if (uri == null || !isPairingLink(uri)) return null
            return try {
                val relayUrl = uri.getQueryParameter("u")?.trim().orEmpty()
                val token = uri.getQueryParameter("t").orEmpty()
                val encodedKey = uri.getQueryParameter("k")?.trim().orEmpty()
                val pcName = uri.getQueryParameter("n")?.trim().orEmpty().ifEmpty { "PC" }
                val lowerUrl = relayUrl.lowercase()
                if (!lowerUrl.startsWith("ws://") && !lowerUrl.startsWith("wss://")) return null
                val key = Base64.decode(encodedKey, Base64.URL_SAFE or Base64.NO_PADDING or Base64.NO_WRAP)
                if (key.size != RelayCrypto.KEY_BYTES) return null
                Pairing(relayUrl, token, key, pcName)
            } catch (invalid: IllegalArgumentException) {
                null
            } catch (unsupported: UnsupportedOperationException) {
                null
            }
        }
    }
}

class PairingStore(context: Context) {
    private val prefs = context.applicationContext.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)

    fun load(): Pairing? {
        val relayUrl = prefs.getString(KEY_RELAY_URL, null) ?: return null
        val encodedKey = prefs.getString(KEY_SECRET, null) ?: return null
        val key = try {
            Base64.decode(encodedKey, Base64.NO_WRAP)
        } catch (invalid: IllegalArgumentException) {
            return null
        }
        if (key.size != RelayCrypto.KEY_BYTES) return null
        return Pairing(
            relayUrl = relayUrl,
            token = prefs.getString(KEY_TOKEN, "").orEmpty(),
            key = key,
            pcName = prefs.getString(KEY_PC_NAME, "PC").orEmpty(),
        )
    }

    val isPaired: Boolean get() = prefs.contains(KEY_RELAY_URL) && prefs.contains(KEY_SECRET)

    fun save(pairing: Pairing) {
        prefs.edit()
            .putString(KEY_RELAY_URL, pairing.relayUrl)
            .putString(KEY_TOKEN, pairing.token)
            .putString(KEY_SECRET, Base64.encodeToString(pairing.key, Base64.NO_WRAP))
            .putString(KEY_PC_NAME, pairing.pcName)
            .commit()
    }

    fun clear() {
        prefs.edit()
            .remove(KEY_RELAY_URL)
            .remove(KEY_TOKEN)
            .remove(KEY_SECRET)
            .remove(KEY_PC_NAME)
            .remove(KEY_SEEN_NOTES)
            .commit()
    }

    var pcName: String
        get() = prefs.getString(KEY_PC_NAME, "PC").orEmpty()
        set(value) {
            if (value.isNotBlank() && value != pcName) prefs.edit().putString(KEY_PC_NAME, value).apply()
        }

    val relayUrl: String get() = prefs.getString(KEY_RELAY_URL, "").orEmpty()

    val deviceId: String
        get() = synchronized(PairingStore::class.java) {
            prefs.getString(KEY_DEVICE_ID, null) ?: RelayCrypto.randomHex(DEVICE_ID_BYTES).also { generated ->
                prefs.edit().putString(KEY_DEVICE_ID, generated).commit()
            }
        }

    fun seenNoteIds(): Set<String> = synchronized(PairingStore::class.java) { readSeenNoteIds() }

    fun rememberNoteIds(noteIds: Collection<String>) {
        if (noteIds.isEmpty()) return
        synchronized(PairingStore::class.java) {
            val seen = readSeenNoteIds()
            seen.addAll(noteIds)
            val trimmed = seen.toList().takeLast(MAX_SEEN_NOTES)
            prefs.edit().putString(KEY_SEEN_NOTES, trimmed.joinToString("\n")).apply()
        }
    }

    private fun readSeenNoteIds(): LinkedHashSet<String> =
        prefs.getString(KEY_SEEN_NOTES, "").orEmpty().split('\n').filterTo(LinkedHashSet()) { it.isNotEmpty() }

    private companion object {
        const val PREFS_NAME = "berklin"
        const val KEY_RELAY_URL = "u"
        const val KEY_TOKEN = "t"
        const val KEY_SECRET = "k"
        const val KEY_PC_NAME = "n"
        const val KEY_DEVICE_ID = "dev"
        const val KEY_SEEN_NOTES = "seenNotes"
        const val DEVICE_ID_BYTES = 8
        const val MAX_SEEN_NOTES = 300
    }
}
