package dev.zyypj.berklin

import java.security.GeneralSecurityException
import java.security.MessageDigest
import java.security.SecureRandom
import javax.crypto.Cipher
import javax.crypto.spec.GCMParameterSpec
import javax.crypto.spec.SecretKeySpec

class RelayCrypto(key: ByteArray) {
    private val secretKey = SecretKeySpec(key, "AES")

    fun seal(plaintext: String): ByteArray {
        val nonce = ByteArray(NONCE_BYTES)
        secureRandom.nextBytes(nonce)
        val cipher = Cipher.getInstance(TRANSFORMATION)
        cipher.init(Cipher.ENCRYPT_MODE, secretKey, GCMParameterSpec(TAG_BITS, nonce))
        val plainBytes = plaintext.toByteArray(Charsets.UTF_8)
        val payload = ByteArray(NONCE_BYTES + cipher.getOutputSize(plainBytes.size))
        System.arraycopy(nonce, 0, payload, 0, NONCE_BYTES)
        val written = cipher.doFinal(plainBytes, 0, plainBytes.size, payload, NONCE_BYTES)
        return if (written == payload.size - NONCE_BYTES) payload else payload.copyOf(NONCE_BYTES + written)
    }

    fun open(payload: ByteArray): String? {
        if (payload.size < NONCE_BYTES + TAG_BYTES) return null
        return try {
            val cipher = Cipher.getInstance(TRANSFORMATION)
            cipher.init(Cipher.DECRYPT_MODE, secretKey, GCMParameterSpec(TAG_BITS, payload, 0, NONCE_BYTES))
            String(cipher.doFinal(payload, NONCE_BYTES, payload.size - NONCE_BYTES), Charsets.UTF_8)
        } catch (failure: GeneralSecurityException) {
            null
        }
    }

    companion object {
        const val KEY_BYTES = 32
        private const val NONCE_BYTES = 12
        private const val TAG_BYTES = 16
        private const val TAG_BITS = TAG_BYTES * 8
        private const val TRANSFORMATION = "AES/GCM/NoPadding"
        private const val ROOM_PREFIX = "berklin-room:"
        private const val ROOM_HEX_LENGTH = 32
        private val HEX_DIGITS = "0123456789abcdef".toCharArray()
        private val secureRandom = SecureRandom()

        fun roomId(key: ByteArray): String {
            val digest = MessageDigest.getInstance("SHA-256")
            digest.update(ROOM_PREFIX.toByteArray(Charsets.UTF_8))
            return toHex(digest.digest(key), ROOM_HEX_LENGTH / 2)
        }

        fun randomHex(byteCount: Int): String {
            val bytes = ByteArray(byteCount)
            secureRandom.nextBytes(bytes)
            return toHex(bytes, byteCount)
        }

        private fun toHex(bytes: ByteArray, byteCount: Int): String {
            val hex = CharArray(byteCount * 2)
            for (index in 0 until byteCount) {
                val value = bytes[index].toInt() and 0xFF
                hex[index * 2] = HEX_DIGITS[value ushr 4]
                hex[index * 2 + 1] = HEX_DIGITS[value and 0x0F]
            }
            return String(hex)
        }
    }
}
