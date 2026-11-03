package cc.hotic.acpira.remote

import android.content.Context
import cc.hotic.acpira.sync.PairingService

/** A build without acpira-mobile (apps/android/scripts/build-net.sh not run): no paired remote access */
object Remote {
  fun service(@Suppress("UNUSED_PARAMETER") context: Context): PairingService? = null
}
