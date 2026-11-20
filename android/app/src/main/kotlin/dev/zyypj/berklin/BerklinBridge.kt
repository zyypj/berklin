package dev.zyypj.berklin

import android.os.Build
import android.webkit.JavascriptInterface
import org.json.JSONException
import org.json.JSONObject

class BerklinBridge(private val activity: MainActivity) {
    private val store = PairingStore(activity)

    @JavascriptInterface
    fun send(json: String) {
        RelayHub.submit(withDeviceIdentity(json))
        if (!RelayHub.serviceRunning) RelayService.start(activity.applicationContext)
    }

    @JavascriptInterface
    fun config(): String {
        val paired = store.isPaired
        return JSONObject()
            .put("paired", paired)
            .put("pcName", if (paired) store.pcName else JSONObject.NULL)
            .put("relayUrl", if (paired) store.relayUrl else JSONObject.NULL)
            .put("dev", store.deviceId)
            .toString()
    }

    @JavascriptInterface
    fun pair(link: String): Boolean = activity.applyPairingLink(link, reloadPage = false)

    @JavascriptInterface
    fun unpair() {
        activity.forgetPairing()
    }

    @JavascriptInterface
    fun batterySettings() {
        activity.runOnUiThread { activity.openBatteryOptimizationSettings() }
    }

    @JavascriptInterface
    fun appVersion(): String = BuildConfig.VERSION_NAME

    @JavascriptInterface
    fun checkUpdate() {
        Updater.check(activity.applicationContext, force = true)
    }

    @JavascriptInterface
    fun installUpdate() {
        activity.runOnUiThread { activity.startUpdateInstall() }
    }

    private fun withDeviceIdentity(json: String): String {
        if (!json.contains("\"hello\"")) return json
        return try {
            val request = JSONObject(json)
            if (request.optString("op") != "hello") return json
            request.put("dev", store.deviceId)
            if (request.optString("platform").isEmpty()) request.put("platform", "android")
            if (request.optString("name").isEmpty()) request.put("name", Build.MODEL ?: "Android")
            request.toString()
        } catch (malformed: JSONException) {
            json
        }
    }
}
