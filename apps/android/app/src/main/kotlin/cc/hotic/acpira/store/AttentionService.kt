package cc.hotic.acpira.store

import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.IBinder
import androidx.core.app.NotificationCompat
import androidx.core.app.ServiceCompat
import cc.hotic.acpira.MainActivity
import cc.hotic.acpira.R
import cc.hotic.acpira.i18n.t

/**
 * Holds the app in the foreground while a turn it follows still runs after the app left the screen, so the wire stays
 * up and [Attention] can say when the turn needs the user or ends; it goes as soon as nothing runs (MainActivity). The
 * process does nothing else meanwhile: the wire and the list live in the app model as when the app is in front
 */
class AttentionService : Service() {
  override fun onBind(intent: Intent?): IBinder? = null

  override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
    Attention.channels(this)
    val count = intent?.getIntExtra(EXTRA_RUNNING, 1) ?: 1
    val title = intent?.getStringExtra(EXTRA_TITLE) ?: getString(R.string.app_name)
    val open = PendingIntent.getActivity(
      this, 0, Intent(this, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP), PendingIntent.FLAG_IMMUTABLE,
    )
    val notice = NotificationCompat.Builder(this, Attention.CHANNEL_LINK)
      .setSmallIcon(R.drawable.ic_notify)
      .setContentTitle(title)
      .setContentText(t("subagents.running", "n" to count))
      .setContentIntent(open)
      .setOngoing(true)
      .setSilent(true)
      .setPriority(NotificationCompat.PRIORITY_MIN)
      .build()
    // A start the system refuses (background start limits) leaves the app as before: the wire closes after its grace
    runCatching {
      ServiceCompat.startForeground(this, NOTICE, notice, ServiceInfo.FOREGROUND_SERVICE_TYPE_REMOTE_MESSAGING)
    }.onFailure { stopSelf() }
    return START_NOT_STICKY
  }

  companion object {
    private const val NOTICE = 2
    private const val EXTRA_RUNNING = "running"
    private const val EXTRA_TITLE = "title"

    /** Shows or updates the service's notice: [running] turns on [title] (the computer's name) */
    fun start(context: Context, running: Int, title: String?): Boolean = runCatching {
      context.startForegroundService(Intent(context, AttentionService::class.java).putExtra(EXTRA_RUNNING, running).putExtra(EXTRA_TITLE, title))
    }.onFailure { android.util.Log.w("acpira", "running-turn service refused: $it") }.isSuccess

    fun stop(context: Context) {
      context.stopService(Intent(context, AttentionService::class.java))
    }
  }
}
