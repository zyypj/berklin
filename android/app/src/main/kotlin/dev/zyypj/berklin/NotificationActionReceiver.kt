package dev.zyypj.berklin

import android.app.RemoteInput
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import org.json.JSONObject

class NotificationActionReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action != ACTION_RESOLVE) return
        val pid = intent.getStringExtra(EXTRA_PID) ?: return
        val tag = intent.getStringExtra(EXTRA_TAG)
        val request = buildResolveRequest(intent, pid)
        if (request != null) {
            RelayHub.submitNativeRequest(request)
            RelayService.start(context)
        }
        if (tag != null && request != null) ClaudeNotifier(context).cancelByTag(tag)
    }

    private fun buildResolveRequest(intent: Intent, pid: String): JSONObject? {
        val request = JSONObject().put("op", "pending.resolve").put("pid", pid)
        val question = intent.getStringExtra(EXTRA_QUESTION).orEmpty()
        when (intent.getStringExtra(EXTRA_CHOICE)) {
            CHOICE_ALLOW -> request.put("allow", true)
            CHOICE_DENY -> request.put("allow", false)
            CHOICE_OPTION -> {
                val label = intent.getStringExtra(EXTRA_LABEL) ?: return null
                request.put("answers", JSONObject().put(question, label))
            }
            CHOICE_REPLY -> {
                val reply = RemoteInput.getResultsFromIntent(intent)
                    ?.getCharSequence(KEY_REPLY_TEXT)
                    ?.toString()
                    ?.trim()
                if (reply.isNullOrEmpty()) return null
                if (intent.getStringExtra(EXTRA_PENDING_KIND) == "question") {
                    request.put("answers", JSONObject().put(question, reply))
                } else {
                    request.put("text", reply)
                }
            }
            else -> return null
        }
        return request
    }

    companion object {
        const val ACTION_RESOLVE = "dev.zyypj.berklin.action.RESOLVE_PENDING"
        const val EXTRA_PID = "pid"
        const val EXTRA_TAG = "tag"
        const val EXTRA_PENDING_KIND = "pendingKind"
        const val EXTRA_CHOICE = "choice"
        const val EXTRA_QUESTION = "question"
        const val EXTRA_LABEL = "label"
        const val KEY_REPLY_TEXT = "replyText"
        const val CHOICE_ALLOW = "allow"
        const val CHOICE_DENY = "deny"
        const val CHOICE_OPTION = "option"
        const val CHOICE_REPLY = "reply"
    }
}
