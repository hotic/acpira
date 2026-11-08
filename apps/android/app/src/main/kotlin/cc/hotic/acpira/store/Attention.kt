package cc.hotic.acpira.store

import android.Manifest
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import androidx.core.content.ContextCompat
import cc.hotic.acpira.MainActivity
import cc.hotic.acpira.R
import cc.hotic.acpira.i18n.t
import cc.hotic.acpira.model.SessionState
import cc.hotic.acpira.model.SessionSummary

/**
 * Notifications from the computer the app follows, the Android side of the iOS app's PushCenter. iOS hears them from
 * the gateway through APNs while the app is suspended; Android has no such channel (no Firebase project, and phones
 * sold without Google services), so they come from the list the wire keeps: a session that turned to waiting, or whose
 * turn ended unread, posts the desktop's attention notice (`host.attention.*`). While a turn still runs as the app
 * leaves, [AttentionService] keeps the process and its wire up until nothing runs any more.
 */
class Attention(private val context: Context) {
  /** What each session was in the list last seen; null until a model's first list, which is only recorded */
  private var seen: Map<String, Mark>? = null
  private var model: AppModel? = null

  private data class Mark(val waiting: Boolean, val unread: Boolean)

  private fun mark(s: SessionSummary) = Mark(
    waiting = s.state == SessionState.waiting,
    unread = s.unread == true || s.state == SessionState.unread,
  )

  /**
   * A new list from [model]. The first list of a model is the baseline: what changed while no wire listened is not
   * news the user can act on now. The chat on screen while the app is in front never notifies, a banner would only
   * repeat it (PushCenter `showing`)
   */
  fun update(model: AppModel, sessions: List<SessionSummary>, shown: String?) {
    val before = seen.takeIf { this.model === model }
    this.model = model
    seen = sessions.associate { it.id to mark(it) }
    if (before == null || !allowed(context)) return
    for (s in sessions) {
      val was = before[s.id] ?: continue
      val now = mark(s)
      if (s.id == shown) continue
      val agent = model.agent(s.agent).name
      when {
        now.waiting && !was.waiting -> post(s, model.computer, t("host.attention.waiting", "agent" to agent))
        now.unread && !was.unread -> post(s, model.computer, t("host.attention.done", "agent" to agent))
      }
    }
  }

  /** A notice gone stale: the session was opened, so its card in the shade says nothing new */
  fun clear(session: String) = NotificationManagerCompat.from(context).cancel(session, NOTICE)

  /** [computer]: the paired computer the session lives on, so a tap follows that computer first (null: a development engine) */
  private fun post(s: SessionSummary, computer: String?, text: String) {
    channels(context)
    val open = Intent(context, MainActivity::class.java)
      .setAction(ACTION_OPEN)
      .putExtra(EXTRA_SESSION, s.id)
      .putExtra(EXTRA_COMPUTER, computer)
      .addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP)
    val tap = PendingIntent.getActivity(context, "$computer/${s.id}".hashCode(), open, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
    val notice = NotificationCompat.Builder(context, CHANNEL_ATTENTION)
      .setSmallIcon(R.drawable.ic_notify)
      .setContentTitle(s.title)
      .setContentText(text)
      .setContentIntent(tap)
      .setAutoCancel(true)
      .setCategory(NotificationCompat.CATEGORY_MESSAGE)
      .setPriority(NotificationCompat.PRIORITY_HIGH)
      .build()
    runCatching { NotificationManagerCompat.from(context).notify(s.id, NOTICE, notice) }
  }

  companion object {
    const val ACTION_OPEN = "cc.hotic.acpira.OPEN_SESSION"
    const val EXTRA_SESSION = "session"
    const val EXTRA_COMPUTER = "computer"
    private const val NOTICE = 1
    const val CHANNEL_ATTENTION = "attention"
    const val CHANNEL_LINK = "link"

    /** Notifications may be posted: granted on Android 13+, on by default before it, and not switched off for the app */
    fun allowed(context: Context): Boolean {
      val granted = Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU ||
        ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) == PackageManager.PERMISSION_GRANTED
      return granted && NotificationManagerCompat.from(context).areNotificationsEnabled()
    }

    /** The two channels: the notices themselves, and the quiet one the running-turn service shows */
    fun channels(context: Context) {
      val manager = context.getSystemService(NotificationManager::class.java) ?: return
      manager.createNotificationChannel(
        NotificationChannel(CHANNEL_ATTENTION, t("settings.notifications.title"), NotificationManager.IMPORTANCE_HIGH),
      )
      manager.createNotificationChannel(
        NotificationChannel(CHANNEL_LINK, t("mobile.state.background"), NotificationManager.IMPORTANCE_MIN).apply { setShowBadge(false) },
      )
    }
  }
}
