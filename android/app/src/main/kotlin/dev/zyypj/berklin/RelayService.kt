package dev.zyypj.berklin

import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.net.ConnectivityManager
import android.net.Network
import android.os.Build
import android.os.Handler
import android.os.HandlerThread
import android.os.IBinder
import android.os.SystemClock
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import okio.ByteString
import okio.ByteString.Companion.toByteString
import org.json.JSONArray
import org.json.JSONException
import org.json.JSONObject
import java.util.concurrent.TimeUnit
import kotlin.random.Random

class RelayService : Service() {
    private lateinit var store: PairingStore
    private lateinit var notifier: ClaudeNotifier
    private lateinit var relayThread: HandlerThread
    private lateinit var relayHandler: Handler
    private lateinit var connectivity: ConnectivityManager

    private val httpClient: OkHttpClient by lazy {
        OkHttpClient.Builder()
            .pingInterval(PING_INTERVAL_SECONDS, TimeUnit.SECONDS)
            .connectTimeout(CONNECT_TIMEOUT_SECONDS, TimeUnit.SECONDS)
            .readTimeout(0, TimeUnit.MILLISECONDS)
            .writeTimeout(WRITE_TIMEOUT_SECONDS, TimeUnit.SECONDS)
            .retryOnConnectionFailure(false)
            .build()
    }

    private var pairing: Pairing? = null
    @Volatile
    private var crypto: RelayCrypto? = null

    @Volatile
    private var destroyed = false
    private var socket: WebSocket? = null
    private var socketGeneration = 0
    private var socketOpen = false
    private var connecting = false
    private var agentOnline: Boolean? = null
    private var failedAttempts = 0
    private var helloId = -1L
    private var currentNetwork: Network? = null
    private var shownConnectionKey: String? = null
    private var networkCallbackRegistered = false

    private val reconnectRunnable = Runnable { connect() }

    private val updateCheckRunnable = object : Runnable {
        override fun run() {
            Updater.check(this@RelayService, force = false)
            relayHandler.postDelayed(this, Updater.PERIODIC_CHECK_INTERVAL_MS)
        }
    }

    private val networkCallback = object : ConnectivityManager.NetworkCallback() {
        override fun onAvailable(network: Network) {
            relayHandler.post { onNetworkAvailable(network) }
        }

        override fun onLost(network: Network) {
            relayHandler.post { onNetworkLost(network) }
        }
    }

    override fun onCreate() {
        super.onCreate()
        store = PairingStore(this)
        notifier = ClaudeNotifier(this)
        notifier.ensureChannels()
        connectivity = getSystemService(ConnectivityManager::class.java)
        relayThread = HandlerThread("berklin-relay").also { it.start() }
        relayHandler = Handler(relayThread.looper)
        RelayHub.serviceRunning = true
        enterForeground()
        relayHandler.post(updateCheckRunnable)
        try {
            connectivity.registerDefaultNetworkCallback(networkCallback)
            networkCallbackRegistered = true
        } catch (failure: RuntimeException) {
            networkCallbackRegistered = false
        }
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        enterForeground()
        if (!store.isPaired) {
            relayHandler.post { shutdownSocket() }
            stopSelf()
            return START_NOT_STICKY
        }
        val forceReload = intent?.action == ACTION_RELOAD
        relayHandler.post { ensureConnected(forceReload) }
        return START_STICKY
    }

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onDestroy() {
        destroyed = true
        RelayHub.serviceRunning = false
        relayHandler.removeCallbacks(updateCheckRunnable)
        if (networkCallbackRegistered) {
            try {
                connectivity.unregisterNetworkCallback(networkCallback)
            } catch (ignored: RuntimeException) {
                networkCallbackRegistered = false
            }
        }
        relayHandler.post {
            shutdownSocket()
            notifier.cancelConnection()
            httpClient.dispatcher.executorService.shutdown()
            httpClient.connectionPool.evictAll()
            relayThread.quitSafely()
        }
        super.onDestroy()
    }

    private fun enterForeground() {
        val notification = notifier.connectionNotification(reconnectingTitle(), null)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
            startForeground(ClaudeNotifier.CONNECTION_NOTIFICATION_ID, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE)
        } else {
            startForeground(ClaudeNotifier.CONNECTION_NOTIFICATION_ID, notification)
        }
        shownConnectionKey = null
        refreshConnectionNotification()
    }

    private fun ensureConnected(forceReload: Boolean) {
        val stored = store.load()
        if (stored == null) {
            shutdownSocket()
            stopSelf()
            return
        }
        val pairingChanged = pairing?.let { current ->
            current.relayUrl != stored.relayUrl || current.token != stored.token || !current.key.contentEquals(stored.key)
        } ?: true
        if (forceReload || pairingChanged) {
            pairing = stored
            crypto = RelayCrypto(stored.key)
            failedAttempts = 0
            restartSocket()
            return
        }
        if (!socketOpen && !connecting) {
            failedAttempts = 0
            connect()
        }
    }

    private fun connect() {
        relayHandler.removeCallbacks(reconnectRunnable)
        val activePairing = pairing ?: return
        socket?.cancel()
        socketGeneration += 1
        val generation = socketGeneration
        socketOpen = false
        connecting = true
        agentOnline = null
        RelayHub.detachSink()
        publishStatus(RelayState.CONNECTING)
        val request = try {
            Request.Builder().url(activePairing.connectUrl(store.deviceId)).build()
        } catch (invalid: IllegalArgumentException) {
            connecting = false
            publishStatus(RelayState.OFFLINE)
            return
        }
        socket = httpClient.newWebSocket(request, SocketListener(generation))
    }

    private fun restartSocket() {
        failedAttempts = 0
        connect()
    }

    private fun shutdownSocket() {
        relayHandler.removeCallbacks(reconnectRunnable)
        socketGeneration += 1
        socket?.close(NORMAL_CLOSURE, null)
        socket = null
        socketOpen = false
        connecting = false
        agentOnline = null
        RelayHub.detachSink()
        publishStatus(RelayState.OFFLINE)
    }

    private fun scheduleReconnect() {
        relayHandler.removeCallbacks(reconnectRunnable)
        if (pairing == null) return
        val exponent = failedAttempts.coerceAtMost(MAX_BACKOFF_EXPONENT)
        val baseDelay = (INITIAL_BACKOFF_MS shl exponent).coerceAtMost(MAX_BACKOFF_MS)
        val jitter = Random.nextLong(0, baseDelay / JITTER_DIVISOR + 1)
        failedAttempts += 1
        relayHandler.postAtTime(reconnectRunnable, SystemClock.uptimeMillis() + (baseDelay + jitter).coerceAtMost(MAX_BACKOFF_MS))
    }

    private fun onSocketOpen(generation: Int) {
        if (generation != socketGeneration) return
        socketOpen = true
        connecting = false
        failedAttempts = 0
        publishStatus(RelayState.ONLINE)
        greetAgent()
    }

    private fun onSocketGone(generation: Int) {
        if (generation != socketGeneration) return
        socket = null
        socketOpen = false
        connecting = false
        agentOnline = null
        RelayHub.detachSink()
        publishStatus(RelayState.OFFLINE)
        scheduleReconnect()
    }

    private fun onRelayText(generation: Int, text: String) {
        if (generation != socketGeneration) return
        val frame = try {
            JSONObject(text)
        } catch (malformed: JSONException) {
            return
        }
        if (frame.optString("relay") != "agent") return
        val online = frame.optBoolean("online", false)
        val previous = agentOnline
        agentOnline = online
        if (online && previous == false) {
            greetAgent()
        } else if (!online) {
            RelayHub.detachSink()
        }
        publishStatus(RelayState.ONLINE)
    }

    private fun greetAgent() {
        val openSocket = socket ?: return
        val activeCrypto = crypto ?: return
        helloId = RelayHub.nextNativeId()
        val hello = JSONObject()
            .put("id", helloId)
            .put("op", "hello")
            .put("dev", store.deviceId)
            .put("name", deviceName())
            .put("platform", "android")
            .toString()
        val sink: (String) -> Boolean = { json -> openSocket.send(activeCrypto.seal(json).toByteString()) }
        if (!RelayHub.attachSink(sink, hello)) openSocket.cancel()
    }

    private fun onAgentPayload(generation: Int, message: JSONObject) {
        if (generation != socketGeneration) return
        if (message.has("re")) {
            if (message.optLong("re", -1L) == helloId && message.optBoolean("ok", false)) {
                message.optJSONObject("data")?.let(::applySnapshot)
            }
            return
        }
        when (message.optString("ev")) {
            "pending" -> message.optJSONObject("p")?.let(notifier::showPending)
            "pending.done" -> message.optString("pid").takeIf { it.isNotEmpty() }?.let(notifier::cancelPending)
            "note" -> message.optJSONObject("n")?.let { note -> acknowledgeNotes(notifier.showNotes(listOf(note))) }
        }
    }

    private fun applySnapshot(snapshot: JSONObject) {
        snapshot.optJSONObject("pc")?.optString("name")?.takeIf { it.isNotBlank() }?.let { name ->
            store.pcName = name
            refreshConnectionNotification(name)
        }
        notifier.syncPending(snapshot.optJSONArray("pending"))
        val notes = snapshot.optJSONArray("notes") ?: JSONArray()
        val noteList = ArrayList<JSONObject>(notes.length())
        for (index in 0 until notes.length()) notes.optJSONObject(index)?.let(noteList::add)
        acknowledgeNotes(notifier.showNotes(noteList))
    }

    private fun acknowledgeNotes(noteIds: List<String>) {
        if (noteIds.isEmpty()) return
        RelayHub.submitNativeRequest(JSONObject().put("op", "notes.ack").put("ids", JSONArray(noteIds)))
    }

    private fun onNetworkAvailable(network: Network) {
        val previous = currentNetwork
        currentNetwork = network
        if (pairing == null) return
        if (previous != null && previous != network && (socketOpen || connecting)) {
            restartSocket()
        } else if (!socketOpen && !connecting) {
            restartSocket()
        }
    }

    private fun onNetworkLost(network: Network) {
        if (network != currentNetwork) return
        currentNetwork = null
        if (socketOpen || connecting) socket?.cancel()
    }

    private fun publishStatus(state: RelayState) {
        RelayHub.publishStatus(state, state == RelayState.ONLINE && agentOnline == true)
        refreshConnectionNotification()
    }

    private fun refreshConnectionNotification(pcNameOverride: String? = null) {
        if (destroyed) return
        val pcName = pcNameOverride ?: store.pcName
        val title = if (socketOpen) "Conectado ao $pcName" else reconnectingTitle()
        val text = if (socketOpen && agentOnline == false) "$pcName está offline" else null
        val key = "$title|$text"
        if (key == shownConnectionKey) return
        shownConnectionKey = key
        notifier.postConnection(notifier.connectionNotification(title, text))
    }

    private fun reconnectingTitle(): String = "Reconectando…"

    private fun deviceName(): String {
        val model = Build.MODEL.orEmpty().trim()
        val manufacturer = Build.MANUFACTURER.orEmpty().trim()
        return when {
            model.isEmpty() -> "Android"
            manufacturer.isEmpty() || model.startsWith(manufacturer, ignoreCase = true) -> model
            else -> "${manufacturer.replaceFirstChar { it.uppercaseChar() }} $model"
        }
    }

    private inner class SocketListener(private val generation: Int) : WebSocketListener() {
        override fun onOpen(webSocket: WebSocket, response: Response) {
            relayHandler.post { onSocketOpen(generation) }
        }

        override fun onMessage(webSocket: WebSocket, text: String) {
            relayHandler.post { onRelayText(generation, text) }
        }

        override fun onMessage(webSocket: WebSocket, bytes: ByteString) {
            val plaintext = crypto?.open(bytes.toByteArray()) ?: return
            val message = try {
                JSONObject(plaintext)
            } catch (malformed: JSONException) {
                return
            }
            val nativeResponse = message.optLong("re", -1L) >= RelayHub.NATIVE_ID_BASE
            if (!nativeResponse) RelayHub.publishAgentMessage(plaintext)
            if (nativeResponse || message.has("ev")) relayHandler.post { onAgentPayload(generation, message) }
        }

        override fun onClosing(webSocket: WebSocket, code: Int, reason: String) {
            webSocket.close(NORMAL_CLOSURE, null)
        }

        override fun onClosed(webSocket: WebSocket, code: Int, reason: String) {
            relayHandler.post { onSocketGone(generation) }
        }

        override fun onFailure(webSocket: WebSocket, t: Throwable, response: Response?) {
            relayHandler.post { onSocketGone(generation) }
        }
    }

    companion object {
        const val ACTION_RELOAD = "dev.zyypj.berklin.action.RELOAD_PAIRING"
        private const val NORMAL_CLOSURE = 1000
        private const val PING_INTERVAL_SECONDS = 25L
        private const val CONNECT_TIMEOUT_SECONDS = 15L
        private const val WRITE_TIMEOUT_SECONDS = 15L
        private const val INITIAL_BACKOFF_MS = 1_000L
        private const val MAX_BACKOFF_MS = 30_000L
        private const val MAX_BACKOFF_EXPONENT = 5
        private const val JITTER_DIVISOR = 5L

        fun start(context: Context) {
            if (!PairingStore(context).isPaired) return
            launch(context, Intent(context, RelayService::class.java))
        }

        fun reload(context: Context) {
            launch(context, Intent(context, RelayService::class.java).setAction(ACTION_RELOAD))
        }

        fun stop(context: Context) {
            context.stopService(Intent(context, RelayService::class.java))
        }

        private fun launch(context: Context, intent: Intent) {
            try {
                context.startForegroundService(intent)
            } catch (notAllowed: IllegalStateException) {
                RelayHub.serviceRunning = false
            } catch (denied: SecurityException) {
                RelayHub.serviceRunning = false
            }
        }
    }
}
