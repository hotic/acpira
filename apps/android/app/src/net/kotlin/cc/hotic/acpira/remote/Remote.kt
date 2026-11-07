package cc.hotic.acpira.remote

import android.content.Context
import cc.hotic.acpira.sync.PairingService

/** This build carries paired remote access (the `net` source set, over acpira-mobile) */
object Remote {
  fun service(context: Context): PairingService = RemotePairingService(context.applicationContext)
}
