package dev.zyypj.berklin

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.RemoteInput
import android.content.Context
import android.content.Intent
import android.graphics.drawable.Icon
import android.os.Build
import org.json.JSONArray
import org.json.JSONObject
import java.util.concurrent.ConcurrentHashMap

class ClaudeNotifier(context: Context) {
    private val appContext = context.applicationContext
    private val manager = appContext.getSystemService(NotificationManager::class.java)
    private val store = PairingStore(appContext)

    fun ensureChannels() {
        val needsYou = NotificationChannel(CHANNEL_NEEDS_YOU, "Claude precisa de você", NotificationManager.IMPORTANCE_HIGH).apply {
            enableVibration(true)
            setShowBadge(true)
        }
        val notices = NotificationChannel(CHANNEL_NOTICES, "Avisos", NotificationManager.IMPORTANCE_DEFAULT)
        val connection = NotificationChannel(CHANNEL_CONNECTION, "Conexão", NotificationManager.IMPORTANCE_MIN).apply {
            setShowBadge(false)
        }
        manager.createNotificationChannels(listOf(needsYou, notices, connection))
    }

    fun connectionNotification(title: String, text: String?): Notification =
        Notification.Builder(appContext, CHANNEL_CONNECTION)
            .setSmallIcon(R.drawable.ic_stat_berklin)
            .setContentTitle(title)
            .setContentText(text)
            .setOngoing(true)
            .setShowWhen(false)
            .setOnlyAlertOnce(true)
            .setCategory(Notification.CATEGORY_SERVICE)
            .setContentIntent(openAppIntent(null, CONNECTION_REQUEST_CODE))
            .build()

    fun postConnection(notification: Notification) {
        manager.notify(CONNECTION_NOTIFICATION_ID, notification)
    }

    fun cancelConnection() {
        manager.cancel(CONNECTION_NOTIFICATION_ID)
    }

    fun syncPending(pendingList: JSONArray?) {
        val livePids = HashSet<String>()
        if (pendingList != null) {
            for (index in 0 until pendingList.length()) {
                val pending = pendingList.optJSONObject(index) ?: continue
                val pid = pending.optString("pid")
                if (pid.isEmpty()) continue
                livePids.add(pid)
                showPending(pending)
            }
        }
        for (active in activePendingTags()) {
            val pid = active.removePrefix(PENDING_TAG_PREFIX)
            if (pid !in livePids) cancelPending(pid)
        }
        postedPendingSignatures.keys.retainAll(livePids)
    }

    fun showPending(pending: JSONObject) {
        val pid = pending.optString("pid")
        if (pid.isEmpty()) return
        val content = PendingContent.from(pending)
        val tag = PENDING_TAG_PREFIX + pid
        val signature = content.signature
        if (postedPendingSignatures[pid] == signature) return
        if (postedPendingSignatures[pid] == null && activeSignature(tag) == signature) {
            postedPendingSignatures[pid] = signature
            return
        }
        val builder = Notification.Builder(appContext, CHANNEL_NEEDS_YOU)
            .setSmallIcon(R.drawable.ic_stat_berklin)
            .setContentTitle(content.title)
            .setContentText(content.text)
            .setStyle(Notification.BigTextStyle().bigText(content.expandedText))
            .setCategory(Notification.CATEGORY_MESSAGE)
            .setOnlyAlertOnce(true)
            .setAutoCancel(true)
            .setShowWhen(true)
            .setWhen(pending.optLong("ts", System.currentTimeMillis()).takeIf { it > 0 } ?: System.currentTimeMillis())
            .setContentIntent(openAppIntent(pending.optString("sid").ifEmpty { null }, tag.hashCode()))
            .addExtras(android.os.Bundle().apply { putString(EXTRA_SIGNATURE, signature) })
        for ((index, action) in content.actions.withIndex()) {
            builder.addAction(buildAction(action, pid, tag, content.kind, index))
        }
        manager.notify(tag, PENDING_NOTIFICATION_ID, builder.build())
        postedPendingSignatures[pid] = signature
    }

    fun cancelPending(pid: String) {
        postedPendingSignatures.remove(pid)
        manager.cancel(PENDING_TAG_PREFIX + pid, PENDING_NOTIFICATION_ID)
    }

    fun cancelByTag(tag: String) {
        if (tag.startsWith(PENDING_TAG_PREFIX)) {
            cancelPending(tag.removePrefix(PENDING_TAG_PREFIX))
        } else {
            manager.cancel(tag, NOTE_NOTIFICATION_ID)
        }
    }

    fun showNotes(notes: List<JSONObject>): List<String> {
        if (notes.isEmpty()) return emptyList()
        val seen = store.seenNoteIds()
        val noteIds = ArrayList<String>(notes.size)
        for (note in notes) {
            val nid = note.optString("nid")
            if (nid.isEmpty()) continue
            noteIds.add(nid)
            if (nid in seen) continue
            postNote(nid, note)
        }
        store.rememberNoteIds(noteIds)
        return noteIds
    }

    fun cancelAllClaudeNotifications() {
        postedPendingSignatures.clear()
        for (active in manager.activeNotifications) {
            val tag = active.tag ?: continue
            if (tag.startsWith(PENDING_TAG_PREFIX) || tag.startsWith(NOTE_TAG_PREFIX)) manager.cancel(tag, active.id)
        }
    }

    private fun postNote(nid: String, note: JSONObject) {
        val tag = NOTE_TAG_PREFIX + nid
        val title = note.optString("title").ifEmpty { "Berklin" }
        val body = note.optString("body")
        val kind = note.optString("kind")
        val sid = note.optString("sid").ifEmpty { null }
        val channel = if (kind == "info" || kind == "needs") CHANNEL_NEEDS_YOU else CHANNEL_NOTICES
        val builder = Notification.Builder(appContext, channel)
            .setSmallIcon(R.drawable.ic_stat_berklin)
            .setContentTitle(title)
            .setContentText(body)
            .setStyle(Notification.BigTextStyle().bigText(body))
            .setCategory(if (note.optString("kind") == "error") Notification.CATEGORY_ERROR else Notification.CATEGORY_STATUS)
            .setAutoCancel(true)
            .setShowWhen(true)
            .setWhen(note.optLong("ts", System.currentTimeMillis()).takeIf { it > 0 } ?: System.currentTimeMillis())
            .setContentIntent(openAppIntent(sid, tag.hashCode()))
        if (sid != null) builder.addAction(sessionReplyAction(sid, tag))
        manager.notify(tag, NOTE_NOTIFICATION_ID, builder.build())
    }

    private fun sessionReplyAction(sid: String, tag: String): Notification.Action {
        val intent = Intent(appContext, NotificationActionReceiver::class.java)
            .setAction(NotificationActionReceiver.ACTION_REPLY_SESSION)
            .putExtra(NotificationActionReceiver.EXTRA_SID, sid)
            .putExtra(NotificationActionReceiver.EXTRA_TAG, tag)
        val mutableFlag = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) PendingIntent.FLAG_MUTABLE else 0
        val pendingIntent = PendingIntent.getBroadcast(appContext, (tag + "#reply").hashCode(), intent, PendingIntent.FLAG_UPDATE_CURRENT or mutableFlag)
        val remoteInput = RemoteInput.Builder(NotificationActionReceiver.KEY_REPLY_TEXT)
            .setLabel("Responder ao Claude")
            .build()
        return Notification.Action.Builder(Icon.createWithResource(appContext, R.drawable.ic_stat_berklin), "Responder", pendingIntent)
            .addRemoteInput(remoteInput)
            .setAllowGeneratedReplies(false)
            .build()
    }

    private fun buildAction(action: PendingAction, pid: String, tag: String, pendingKind: String, index: Int): Notification.Action {
        val intent = Intent(appContext, NotificationActionReceiver::class.java)
            .setAction(NotificationActionReceiver.ACTION_RESOLVE)
            .putExtra(NotificationActionReceiver.EXTRA_PID, pid)
            .putExtra(NotificationActionReceiver.EXTRA_TAG, tag)
            .putExtra(NotificationActionReceiver.EXTRA_PENDING_KIND, pendingKind)
            .putExtra(NotificationActionReceiver.EXTRA_CHOICE, action.choice)
            .putExtra(NotificationActionReceiver.EXTRA_QUESTION, action.question)
            .putExtra(NotificationActionReceiver.EXTRA_LABEL, action.label)
        val requestCode = (tag + "#" + index).hashCode()
        val icon = Icon.createWithResource(appContext, R.drawable.ic_stat_berklin)
        if (action.choice == NotificationActionReceiver.CHOICE_REPLY) {
            val mutableFlag = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) PendingIntent.FLAG_MUTABLE else 0
            val pendingIntent = PendingIntent.getBroadcast(appContext, requestCode, intent, PendingIntent.FLAG_UPDATE_CURRENT or mutableFlag)
            val remoteInput = RemoteInput.Builder(NotificationActionReceiver.KEY_REPLY_TEXT)
                .setLabel(action.label)
                .build()
            return Notification.Action.Builder(icon, action.label, pendingIntent)
                .addRemoteInput(remoteInput)
                .setAllowGeneratedReplies(false)
                .build()
        }
        val pendingIntent = PendingIntent.getBroadcast(appContext, requestCode, intent, PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
        return Notification.Action.Builder(icon, action.label, pendingIntent).build()
    }

    private fun openAppIntent(sid: String?, requestCode: Int): PendingIntent {
        val intent = Intent(appContext, MainActivity::class.java)
            .setAction(MainActivity.ACTION_OPEN_SESSION)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP)
        if (sid != null) intent.putExtra(MainActivity.EXTRA_SID, sid)
        return PendingIntent.getActivity(appContext, requestCode, intent, PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
    }

    private fun activePendingTags(): List<String> =
        manager.activeNotifications.mapNotNull { active -> active.tag?.takeIf { it.startsWith(PENDING_TAG_PREFIX) } }

    private fun activeSignature(tag: String): String? =
        manager.activeNotifications.firstOrNull { it.tag == tag }?.notification?.extras?.getString(EXTRA_SIGNATURE)

    companion object {
        const val CONNECTION_NOTIFICATION_ID = 1
        private const val PENDING_NOTIFICATION_ID = 2
        private const val NOTE_NOTIFICATION_ID = 3
        private const val CONNECTION_REQUEST_CODE = 1
        private const val CHANNEL_NEEDS_YOU = "needs_you"
        private const val CHANNEL_NOTICES = "notices"
        private const val CHANNEL_CONNECTION = "connection"
        private const val PENDING_TAG_PREFIX = "pending:"
        private const val NOTE_TAG_PREFIX = "note:"
        private const val EXTRA_SIGNATURE = "dev.zyypj.berklin.signature"
        private val postedPendingSignatures = ConcurrentHashMap<String, String>()
    }
}

class PendingAction(val label: String, val choice: String, val question: String?)

class PendingContent(
    val kind: String,
    val title: String,
    val text: String,
    val expandedText: String,
    val actions: List<PendingAction>,
) {
    val signature: String
        get() = buildString {
            append(kind).append('\u0001').append(title).append('\u0001').append(expandedText)
            for (action in actions) append('\u0001').append(action.label)
        }

    companion object {
        private const val MAX_TEXT = 600
        private const val MAX_QUICK_OPTIONS = 3
        private val INPUT_SUMMARY_KEYS = listOf("command", "file_path", "path", "url", "pattern", "query", "description", "prompt")

        fun from(pending: JSONObject): PendingContent {
            val sessionTitle = pending.optString("title").ifEmpty { "Claude" }
            return when (val kind = pending.optString("kind")) {
                "permission" -> permission(kind, sessionTitle, pending)
                "question" -> question(kind, sessionTitle, pending)
                else -> stop(kind.ifEmpty { "stop" }, sessionTitle, pending)
            }
        }

        private fun permission(kind: String, sessionTitle: String, pending: JSONObject): PendingContent {
            val tool = pending.optString("tool").ifEmpty { "Uma ferramenta" }
            val summary = summarizeInput(pending.opt("input"))
            return PendingContent(
                kind = kind,
                title = "$sessionTitle: $tool precisa de permissão",
                text = summary,
                expandedText = summary,
                actions = listOf(
                    PendingAction("Permitir", NotificationActionReceiver.CHOICE_ALLOW, null),
                    PendingAction("Negar", NotificationActionReceiver.CHOICE_DENY, null),
                ),
            )
        }

        private fun question(kind: String, sessionTitle: String, pending: JSONObject): PendingContent {
            val questions = pending.optJSONArray("questions") ?: JSONArray()
            val first = questions.optJSONObject(0)
            val questionText = first?.optString("question").orEmpty()
            val header = first?.optString("header").orEmpty()
            val options = first?.optJSONArray("options") ?: JSONArray()
            val expanded = buildString {
                append(questionText)
                for (index in 0 until options.length()) {
                    val option = options.optJSONObject(index) ?: continue
                    append("\n• ").append(option.optString("label"))
                    val description = option.optString("description")
                    if (description.isNotEmpty()) append(" — ").append(description)
                }
                if (questions.length() > 1) append("\n+").append(questions.length() - 1).append(" pergunta(s) no app")
            }
            val actions = ArrayList<PendingAction>()
            val quickOptions = questions.length() == 1 && first != null && !first.optBoolean("multiSelect", false) &&
                options.length() in 1..MAX_QUICK_OPTIONS
            if (quickOptions) {
                for (index in 0 until options.length()) {
                    val label = options.optJSONObject(index)?.optString("label").orEmpty()
                    if (label.isNotEmpty()) actions.add(PendingAction(label, NotificationActionReceiver.CHOICE_OPTION, questionText))
                }
            }
            actions.add(PendingAction("Responder", NotificationActionReceiver.CHOICE_REPLY, questionText))
            val titleSuffix = if (header.isNotEmpty()) header else "pergunta"
            return PendingContent(
                kind = kind,
                title = "$sessionTitle: $titleSuffix",
                text = questionText.take(MAX_TEXT),
                expandedText = expanded.take(MAX_TEXT * 2),
                actions = actions,
            )
        }

        private fun stop(kind: String, sessionTitle: String, pending: JSONObject): PendingContent {
            val message = pending.optString("message").trim()
            return PendingContent(
                kind = kind,
                title = "$sessionTitle terminou",
                text = message.take(MAX_TEXT),
                expandedText = message.take(MAX_TEXT * 2),
                actions = listOf(PendingAction("Responder", NotificationActionReceiver.CHOICE_REPLY, null)),
            )
        }

        private fun summarizeInput(input: Any?): String {
            val summary = when (input) {
                null, JSONObject.NULL -> ""
                is JSONObject -> INPUT_SUMMARY_KEYS.firstNotNullOfOrNull { key ->
                    input.optString(key).takeIf { input.opt(key) is String && it.isNotBlank() }
                } ?: input.toString()
                else -> input.toString()
            }
            return summary.take(MAX_TEXT)
        }
    }
}
