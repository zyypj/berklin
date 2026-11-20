package dev.zyypj.berklin

import org.json.JSONObject
import java.util.concurrent.CopyOnWriteArraySet
import java.util.concurrent.atomic.AtomicLong

enum class RelayState(val wireName: String) {
    CONNECTING("connecting"),
    ONLINE("online"),
    OFFLINE("offline"),
}

object RelayHub {
    const val NATIVE_ID_BASE = 1_000_000_000L
    private const val MAX_OUTBOX_SIZE = 256

    interface Listener {
        fun onAgentMessage(json: String)
        fun onRelayStatus(statusJson: String)
    }

    private val listeners = CopyOnWriteArraySet<Listener>()
    private val nativeIds = AtomicLong(NATIVE_ID_BASE)
    private val outbox = ArrayDeque<String>()
    private var socketSink: ((String) -> Boolean)? = null

    @Volatile
    var serviceRunning: Boolean = false

    @Volatile
    var statusJson: String = statusJsonOf(RelayState.OFFLINE, false)
        private set

    fun nextNativeId(): Long = nativeIds.incrementAndGet()

    fun addListener(listener: Listener) {
        listeners.add(listener)
    }

    fun removeListener(listener: Listener) {
        listeners.remove(listener)
    }

    fun publishAgentMessage(json: String) {
        for (listener in listeners) listener.onAgentMessage(json)
    }

    fun publishStatus(state: RelayState, agentOnline: Boolean) {
        val json = statusJsonOf(state, agentOnline)
        if (json == statusJson) return
        statusJson = json
        for (listener in listeners) listener.onRelayStatus(json)
    }

    fun submit(json: String) {
        synchronized(this) {
            val delivered = socketSink?.invoke(json) ?: false
            if (!delivered) {
                if (outbox.size >= MAX_OUTBOX_SIZE) outbox.removeFirst()
                outbox.addLast(json)
            }
        }
    }

    fun submitNativeRequest(request: JSONObject) {
        request.put("id", nextNativeId())
        submit(request.toString())
    }

    fun attachSink(sink: (String) -> Boolean, greeting: String): Boolean = synchronized(this) {
        socketSink = null
        if (!sink(greeting)) return false
        while (outbox.isNotEmpty()) {
            if (!sink(outbox.first())) return false
            outbox.removeFirst()
        }
        socketSink = sink
        true
    }

    fun detachSink() {
        synchronized(this) { socketSink = null }
    }

    fun clearOutbox() {
        synchronized(this) { outbox.clear() }
    }

    private fun statusJsonOf(state: RelayState, agentOnline: Boolean): String =
        JSONObject().put("relay", state.wireName).put("agent", agentOnline).toString()
}
