package dev.zyypj.berklin

import android.content.res.AssetManager
import android.net.Uri
import android.webkit.MimeTypeMap
import android.webkit.WebResourceRequest
import android.webkit.WebResourceResponse
import java.io.ByteArrayInputStream
import java.io.IOException

class AppAssetServer(private val assets: AssetManager) {

    fun serves(uri: Uri): Boolean = uri.scheme == SCHEME && uri.host == HOST

    fun respond(request: WebResourceRequest): WebResourceResponse? {
        val uri = request.url
        if (!serves(uri)) return null
        if (request.method != "GET") return failure(405, "Method Not Allowed")
        val rawPath = uri.path.orEmpty().trimStart('/')
        val assetPath = if (rawPath.isEmpty() || rawPath.endsWith("/")) rawPath + "index.html" else rawPath
        if (assetPath.split('/').any { it == ".." }) return failure(403, "Forbidden")
        return try {
            val stream = assets.open(assetPath, AssetManager.ACCESS_STREAMING)
            val mimeType = mimeTypeOf(assetPath)
            val encoding = if (mimeType.startsWith("text/") || mimeType == "application/javascript" || mimeType == "application/json") "utf-8" else null
            WebResourceResponse(mimeType, encoding, 200, "OK", RESPONSE_HEADERS, stream)
        } catch (missing: IOException) {
            failure(404, "Not Found")
        }
    }

    private fun failure(code: Int, reason: String): WebResourceResponse =
        WebResourceResponse("text/plain", "utf-8", code, reason, RESPONSE_HEADERS, ByteArrayInputStream(ByteArray(0)))

    private fun mimeTypeOf(path: String): String {
        val extension = path.substringAfterLast('.', "").lowercase()
        return KNOWN_TYPES[extension]
            ?: MimeTypeMap.getSingleton().getMimeTypeFromExtension(extension)
            ?: "application/octet-stream"
    }

    companion object {
        const val SCHEME = "https"
        const val HOST = "appassets.androidplatform.net"
        const val ORIGIN = "$SCHEME://$HOST"
        private val RESPONSE_HEADERS = mapOf("Cache-Control" to "no-cache")
        private val KNOWN_TYPES = mapOf(
            "html" to "text/html",
            "htm" to "text/html",
            "js" to "application/javascript",
            "mjs" to "application/javascript",
            "css" to "text/css",
            "json" to "application/json",
            "map" to "application/json",
            "svg" to "image/svg+xml",
            "png" to "image/png",
            "jpg" to "image/jpeg",
            "jpeg" to "image/jpeg",
            "webp" to "image/webp",
            "gif" to "image/gif",
            "ico" to "image/x-icon",
            "woff" to "font/woff",
            "woff2" to "font/woff2",
            "ttf" to "font/ttf",
            "wasm" to "application/wasm",
            "txt" to "text/plain",
        )
    }
}
