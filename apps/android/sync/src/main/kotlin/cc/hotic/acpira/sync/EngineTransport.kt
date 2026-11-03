package cc.hotic.acpira.sync

import kotlinx.coroutines.channels.awaitClose
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.callbackFlow
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import okio.ByteString
import java.util.concurrent.TimeUnit

/**
 * One wire to an engine, carrying text frames of envelopes. The development WebSocket to `acpira --ws` is the first;
 * the paired remote connection plugs in behind the same interface
 */
interface EngineTransport {
  /** Frames from the engine until the wire closes (completes) or fails (throws). Collected once, before the first send */
  fun frames(): Flow<String>
  /** False when the wire is already gone */
  fun send(text: String): Boolean
  fun close()
}

/**
 * `acpira --ws` (apps/ios/scripts/engine.sh, `pnpm harness`), loopback only: the token rides in the query and the engine
 * drops a handshake without a loopback `Origin` of its own port (rust/crates/acpira-host/src/sidecar/ws.rs). The
 * emulator reaches the computer's loopback through `adb reverse`, so the app dials 127.0.0.1 as the iOS simulator does
 */
class WebSocketTransport(
  private val port: Int,
  private val token: String,
  private val host: String = "127.0.0.1",
  private val client: OkHttpClient = defaultClient,
) : EngineTransport {
  private var socket: WebSocket? = null
  @Volatile private var closedHere = false
  /** Sent before the socket exists (hello goes before the first frame is read); written once it does */
  private val pending = ArrayList<String>()

  override fun frames(): Flow<String> = callbackFlow {
    val request = Request.Builder()
      .url("http://$host:$port/ws?token=$token")
      .header("Origin", "http://$host:$port")
      .build()
    val opened = client.newWebSocket(request, object : WebSocketListener() {
      override fun onMessage(webSocket: WebSocket, text: String) {
        trySend(text)
      }
      override fun onMessage(webSocket: WebSocket, bytes: ByteString) {
        trySend(bytes.utf8())
      }
      override fun onClosing(webSocket: WebSocket, code: Int, reason: String) {
        webSocket.close(1000, null)
        channel.close()
      }
      override fun onClosed(webSocket: WebSocket, code: Int, reason: String) {
        channel.close()
      }
      override fun onFailure(webSocket: WebSocket, t: Throwable, response: Response?) {
        // A close this side asked for ends the stream quietly; anything else is the wire failing
        if (closedHere) channel.close() else channel.close(t)
      }
    })
    // OkHttp queues what is sent before the handshake completes, so the backlog goes straight in
    synchronized(this@WebSocketTransport) {
      socket = opened
      pending.forEach { opened.send(it) }
      pending.clear()
    }
    awaitClose { opened.cancel() }
  }

  override fun send(text: String): Boolean = synchronized(this) {
    if (closedHere) return false
    val open = socket ?: run { pending.add(text); return true }
    open.send(text)
  }

  override fun close() {
    val open = synchronized(this) {
      closedHere = true
      socket
    }
    open?.close(1000, null)
  }

  companion object {
    /** No read timeout: the wire sits quiet between turns. The engine closes a frame over its own cap, not OkHttp */
    val defaultClient: OkHttpClient by lazy {
      OkHttpClient.Builder().readTimeout(0, TimeUnit.MILLISECONDS).build()
    }
  }
}
