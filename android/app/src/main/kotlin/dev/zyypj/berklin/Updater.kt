package dev.zyypj.berklin

import android.app.Activity
import android.app.Notification
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.ActivityNotFoundException
import android.content.Context
import android.content.Intent
import android.content.pm.PackageInstaller
import android.net.Uri
import android.provider.Settings
import okhttp3.OkHttpClient
import okhttp3.Request
import org.json.JSONException
import org.json.JSONObject
import java.io.File
import java.io.IOException
import java.util.concurrent.CopyOnWriteArraySet
import java.util.concurrent.ExecutorService
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean

class UpdateState(
    val latest: String?,
    val available: Boolean,
    val downloaded: Boolean,
    val error: String?,
) {
    fun toJson(): String {
        val json = JSONObject()
            .put("current", BuildConfig.VERSION_NAME)
            .put("latest", latest ?: JSONObject.NULL)
            .put("available", available)
            .put("downloaded", downloaded)
        if (error != null) json.put("error", error)
        return json.toString()
    }
}

object Updater {
    const val PERIODIC_CHECK_INTERVAL_MS = 6L * 60 * 60 * 1000
    private const val RELEASES_URL = "https://api.github.com/repos/zyypj/berklin/releases/latest"
    private const val USER_AGENT = "Berklin-Android"
    private const val GITHUB_ACCEPT = "application/vnd.github+json"
    private const val MIN_AUTOMATIC_CHECK_GAP_MS = 30L * 60 * 1000
    private const val PREFS_NAME = "berklin_update"
    private const val KEY_LAST_CHECK = "lastCheck"
    private const val KEY_NOTIFIED_VERSION = "notifiedVersion"
    private const val UPDATES_DIRECTORY = "updates"
    private const val UPDATE_NOTIFICATION_TAG = "update"
    private const val UPDATE_NOTIFICATION_ID = 4
    private const val NOTICES_CHANNEL = "notices"
    private const val HTTP_NOT_FOUND = 404

    fun interface Listener {
        fun onUpdateState(stateJson: String)
    }

    private val listeners = CopyOnWriteArraySet<Listener>()
    private val checking = AtomicBoolean(false)
    private val worker: ExecutorService by lazy { Executors.newSingleThreadExecutor { task -> Thread(task, "berklin-updater") } }
    private val httpClient: OkHttpClient by lazy {
        OkHttpClient.Builder()
            .connectTimeout(20, TimeUnit.SECONDS)
            .readTimeout(60, TimeUnit.SECONDS)
            .followRedirects(true)
            .followSslRedirects(true)
            .build()
    }

    @Volatile
    var state: UpdateState? = null
        private set

    @Volatile
    private var downloadedApk: File? = null

    fun addListener(listener: Listener) {
        listeners.add(listener)
    }

    fun removeListener(listener: Listener) {
        listeners.remove(listener)
    }

    fun check(context: Context, force: Boolean) {
        val appContext = context.applicationContext
        val prefs = appContext.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)
        val now = System.currentTimeMillis()
        if (!force && now - prefs.getLong(KEY_LAST_CHECK, 0L) < MIN_AUTOMATIC_CHECK_GAP_MS) return
        if (!checking.compareAndSet(false, true)) return
        worker.execute {
            try {
                runCheck(appContext)
                prefs.edit().putLong(KEY_LAST_CHECK, System.currentTimeMillis()).apply()
            } finally {
                checking.set(false)
            }
        }
    }

    fun install(activity: Activity): Boolean {
        val apk = downloadedApk?.takeIf { it.isFile } ?: findDownloadedApk(activity) ?: return false
        if (!activity.packageManager.canRequestPackageInstalls()) {
            try {
                activity.startActivity(
                    Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES, Uri.parse("package:${activity.packageName}")),
                )
            } catch (missing: ActivityNotFoundException) {
                publish(state?.let { UpdateState(it.latest, it.available, it.downloaded, "Permissão de instalação indisponível") })
            }
            return true
        }
        val appContext = activity.applicationContext
        worker.execute { commitInstallSession(appContext, apk) }
        return true
    }

    fun reportInstallFailure(message: String?) {
        val current = state
        publish(UpdateState(current?.latest, current?.available ?: false, current?.downloaded ?: false, message ?: "Falha na instalação"))
    }

    private fun runCheck(context: Context) {
        val release = try {
            fetchLatestRelease()
        } catch (failure: IOException) {
            publish(UpdateState(state?.latest, state?.available ?: false, state?.downloaded ?: false, failure.message ?: "Falha de rede"))
            return
        } catch (failure: JSONException) {
            publish(UpdateState(state?.latest, false, false, "Resposta inválida do GitHub"))
            return
        }
        if (release == null) {
            downloadedApk = null
            publish(UpdateState(null, false, false, null))
            return
        }
        if (!isNewer(release.version, BuildConfig.VERSION_NAME)) {
            downloadedApk = null
            cleanUpdates(context, keep = null)
            cancelUpdateNotification(context)
            publish(UpdateState(release.version, false, false, null))
            return
        }
        if (release.apkUrl == null) {
            publish(UpdateState(release.version, true, false, "Release sem APK"))
            return
        }
        val target = File(File(context.cacheDir, UPDATES_DIRECTORY), "berklin-${release.version}.apk")
        cleanUpdates(context, keep = target)
        val alreadyDownloaded = target.isFile && release.apkSize > 0 && target.length() == release.apkSize
        if (!alreadyDownloaded) publish(UpdateState(release.version, true, false, null))
        val ready = alreadyDownloaded || try {
            download(release.apkUrl, release.apkSize, target)
            true
        } catch (failure: IOException) {
            publish(UpdateState(release.version, true, false, failure.message ?: "Falha no download"))
            false
        }
        if (!ready) return
        downloadedApk = target
        notifyUpdateReady(context, release.version)
        publish(UpdateState(release.version, true, true, null))
    }

    private class LatestRelease(val version: String, val apkUrl: String?, val apkSize: Long)

    private fun fetchLatestRelease(): LatestRelease? {
        val request = Request.Builder()
            .url(RELEASES_URL)
            .header("User-Agent", USER_AGENT)
            .header("Accept", GITHUB_ACCEPT)
            .build()
        httpClient.newCall(request).execute().use { response ->
            if (response.code == HTTP_NOT_FOUND) return null
            if (!response.isSuccessful) throw IOException("GitHub respondeu ${response.code}")
            val body = response.body?.string() ?: throw IOException("Resposta vazia do GitHub")
            val release = JSONObject(body)
            val version = release.optString("tag_name").trim().removePrefix("v").removePrefix("V")
            if (version.isEmpty()) return null
            val assets = release.optJSONArray("assets")
            var apkUrl: String? = null
            var apkSize = -1L
            if (assets != null) {
                for (index in 0 until assets.length()) {
                    val asset = assets.optJSONObject(index) ?: continue
                    if (asset.optString("name").endsWith(".apk", ignoreCase = true)) {
                        apkUrl = asset.optString("browser_download_url").ifEmpty { null }
                        apkSize = asset.optLong("size", -1L)
                        break
                    }
                }
            }
            return LatestRelease(version, apkUrl, apkSize)
        }
    }

    private fun download(url: String, expectedSize: Long, target: File) {
        target.parentFile?.mkdirs()
        val partial = File(target.parentFile, target.name + ".part")
        val request = Request.Builder().url(url).header("User-Agent", USER_AGENT).build()
        httpClient.newCall(request).execute().use { response ->
            if (!response.isSuccessful) throw IOException("Download respondeu ${response.code}")
            val body = response.body ?: throw IOException("Download vazio")
            partial.outputStream().use { output -> body.byteStream().use { input -> input.copyTo(output) } }
        }
        if (expectedSize > 0 && partial.length() != expectedSize) {
            partial.delete()
            throw IOException("Download incompleto")
        }
        if (target.exists()) target.delete()
        if (!partial.renameTo(target)) {
            partial.delete()
            throw IOException("Não foi possível salvar a atualização")
        }
    }

    private fun findDownloadedApk(context: Context): File? {
        val directory = File(context.cacheDir, UPDATES_DIRECTORY)
        return directory.listFiles()
            ?.filter { it.isFile && it.name.startsWith("berklin-") && it.name.endsWith(".apk") }
            ?.filter { isNewer(it.name.removePrefix("berklin-").removeSuffix(".apk"), BuildConfig.VERSION_NAME) }
            ?.maxByOrNull { it.lastModified() }
            ?.also { downloadedApk = it }
    }

    private fun cleanUpdates(context: Context, keep: File?) {
        val directory = File(context.cacheDir, UPDATES_DIRECTORY)
        directory.listFiles()?.forEach { file ->
            if (file != keep) file.delete()
        }
    }

    private fun commitInstallSession(context: Context, apk: File) {
        val installer = context.packageManager.packageInstaller
        val params = PackageInstaller.SessionParams(PackageInstaller.SessionParams.MODE_FULL_INSTALL).apply {
            setAppPackageName(context.packageName)
            setSize(apk.length())
        }
        var sessionId = -1
        try {
            sessionId = installer.createSession(params)
            installer.openSession(sessionId).use { session ->
                session.openWrite("berklin.apk", 0, apk.length()).use { output ->
                    apk.inputStream().use { input -> input.copyTo(output) }
                    session.fsync(output)
                }
                val callback = Intent(context, UpdateInstallReceiver::class.java).setAction(UpdateInstallReceiver.ACTION_INSTALL_STATUS)
                val pendingIntent = PendingIntent.getBroadcast(
                    context,
                    sessionId,
                    callback,
                    PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_MUTABLE,
                )
                session.commit(pendingIntent.intentSender)
            }
        } catch (failure: IOException) {
            if (sessionId >= 0) installer.abandonSession(sessionId)
            reportInstallFailure(failure.message)
        } catch (failure: SecurityException) {
            if (sessionId >= 0) installer.abandonSession(sessionId)
            reportInstallFailure(failure.message)
        }
    }

    private fun notifyUpdateReady(context: Context, version: String) {
        val prefs = context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)
        if (prefs.getString(KEY_NOTIFIED_VERSION, null) == version) return
        ClaudeNotifier(context).ensureChannels()
        val manager = context.getSystemService(NotificationManager::class.java)
        val installIntent = Intent(context, MainActivity::class.java)
            .setAction(MainActivity.ACTION_INSTALL_UPDATE)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP)
        val contentIntent = PendingIntent.getActivity(
            context,
            UPDATE_NOTIFICATION_TAG.hashCode(),
            installIntent,
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )
        val notification = Notification.Builder(context, NOTICES_CHANNEL)
            .setSmallIcon(R.drawable.ic_stat_berklin)
            .setContentTitle("Atualização $version disponível")
            .setContentText("Toque para instalar")
            .setAutoCancel(true)
            .setOnlyAlertOnce(true)
            .setContentIntent(contentIntent)
            .build()
        manager.notify(UPDATE_NOTIFICATION_TAG, UPDATE_NOTIFICATION_ID, notification)
        prefs.edit().putString(KEY_NOTIFIED_VERSION, version).apply()
    }

    private fun cancelUpdateNotification(context: Context) {
        context.getSystemService(NotificationManager::class.java).cancel(UPDATE_NOTIFICATION_TAG, UPDATE_NOTIFICATION_ID)
    }

    private fun publish(newState: UpdateState?) {
        if (newState == null) return
        state = newState
        val json = newState.toJson()
        for (listener in listeners) listener.onUpdateState(json)
    }

    fun isNewer(candidate: String, current: String): Boolean {
        val candidateParts = versionParts(candidate)
        val currentParts = versionParts(current)
        for (index in 0 until 3) {
            if (candidateParts[index] != currentParts[index]) return candidateParts[index] > currentParts[index]
        }
        return false
    }

    private fun versionParts(version: String): IntArray {
        val core = version.trim().removePrefix("v").removePrefix("V").substringBefore('-').substringBefore('+')
        val pieces = core.split('.')
        return IntArray(3) { index -> pieces.getOrNull(index)?.trim()?.toIntOrNull() ?: 0 }
    }
}
