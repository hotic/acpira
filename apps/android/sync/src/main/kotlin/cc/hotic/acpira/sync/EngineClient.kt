package cc.hotic.acpira.sync

import cc.hotic.acpira.model.Hello
import cc.hotic.acpira.model.HelloOk
import cc.hotic.acpira.model.HostMsg
import cc.hotic.acpira.model.InitialView
import cc.hotic.acpira.model.PlatformRequest
import cc.hotic.acpira.model.ShellMsg
import cc.hotic.acpira.model.SidecarMsg
import cc.hotic.acpira.model.WebviewMsg
import cc.hotic.acpira.model.WireJson
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.receiveAsFlow
import kotlinx.coroutines.launch
import java.util.UUID

/** What the client hands the app */
sealed interface EngineEvent {
  /** helloOk landed and the view is attached; actions posted before it have gone out */
  data class Connected(val ok: HelloOk) : EngineEvent
  /** A HostMsg for the view */
  data class Host(val message: HostMsg) : EngineEvent
  /** A platform request the app carries out (a toast, a turn's notification) */
  data class Platform(val request: PlatformRequest) : EngineEvent
  /** An envelope that did not decode: a push the app now misses, reported so it never vanishes silently */
  data class Dropped(val reason: String) : EngineEvent
  /** The engine turned the hello down for speaking another protocol version (its own here); Closed follows */
  data class Incompatible(val engineVersion: Int) : EngineEvent
  /** The wire ended (reason null: closed from this side) */
  data class Closed(val reason: String?) : EngineEvent
}

/**
 * The envelope client of one view (test/host-preview/shim.js is the reference, apps/ios AcpiraSync/EngineClient.swift
 * the sibling): hello first, attachView once helloOk lands, actions queued until then and wrapped in webviewMessage
 * after, HostMsgs of the view handed on as events, and platform requests outside what the app carries out answered
 * with an error so the engine never waits them out
 */
class EngineClient(
  private val transport: EngineTransport,
  private val hello: Hello,
  private val viewId: String = "phone",
  /** What the view opens on; Empty never starts a session by itself, which a phone opening on its list wants */
  private val initial: InitialView? = InitialView.Empty,
  /** How long helloOk may take before the wire is given up (an engine that refuses a hello may not close it) */
  private val helloTimeoutMs: Long = HELLO_TIMEOUT_MS,
) {
  /** Buffered without bound, so nothing emitted before the app starts collecting is lost; collected once */
  private val _events = Channel<EngineEvent>(Channel.UNLIMITED)
  val events: Flow<EngineEvent> = _events.receiveAsFlow()
  /** Actions in the order they were posted; drained once the view is attached */
  private val outbox = Channel<WebviewMsg>(Channel.UNLIMITED)
  private var reader: Job? = null
  private var writer: Job? = null
  @Volatile private var helloTimedOut = false
  /** The wire ended: what is still queued goes to [unsent] instead of out */
  @Volatile private var ended = false
  /** What this wire may not have delivered ([unsent]) */
  private val ledger = Ledger()

  private class Ledger {
    /** Posted after the wire ended */
    val late = ArrayList<WebviewMsg>()
    /** Queued but never written */
    val leftover = ArrayList<WebviewMsg>()
    /** Written actions carrying a clientId, newest last, with when (monotonic ms) */
    val written = ArrayDeque<Pair<Long, WebviewMsg>>()
    var endedAt: Long? = null
  }

  /** Says hello and reads the wire until it ends; `events` then carries Closed */
  fun start(scope: CoroutineScope) {
    if (reader != null) return
    reader = scope.launch { run(scope) }
  }

  /**
   * Sends an action in posting order; before the view is attached (`ready` usually comes first) it waits. An action
   * that drives a session gets a clientId here, so a resend on the next wire runs once
   */
  fun post(message: WebviewMsg) {
    val stamped = message.stamped { UUID.randomUUID().toString() }
    if (outbox.trySend(stamped).isFailure) synchronized(ledger) { ledger.late.add(stamped) }
  }

  /**
   * Once the wire ended, what it may not have delivered and is worth sending on the next one (`replayable`), oldest
   * first: clientId actions written shortly before the end, actions still queued, actions posted after it. The engine
   * runs each clientId once, so a resend that did arrive changes nothing
   */
  fun unsent(): List<WebviewMsg> = synchronized(ledger) {
    val end = ledger.endedAt
    val unsure = if (end == null) emptyList() else ledger.written.filter { end - it.first <= UNSURE_WINDOW_MS }.map { it.second }
    (unsure + ledger.leftover + ledger.late).filter { it.replayable }
  }

  fun stop() {
    outbox.close()
    transport.close()
  }

  private suspend fun run(scope: CoroutineScope) {
    var failure: String? = null
    var deadline: Job? = null
    try {
      val frames = transport.frames()
      write(ShellMsg.HelloMsg(hello))
      deadline = scope.launch {
        delay(helloTimeoutMs)
        if (writer == null) {
          helloTimedOut = true
          transport.close()
        }
      }
      frames.collect { frame ->
        // Envelopes end at "\n" alone; JSON leaves U+2028 / U+2029 unescaped inside strings, so never split on them
        for (line in frame.split('\n')) {
          if (line.isBlank()) continue
          receive(line, scope)
        }
      }
    } catch (e: CancellationException) {
      throw e
    } catch (e: Throwable) {
      failure = e.message ?: e.toString()
    }
    deadline?.cancel()
    if (helloTimedOut) failure = "The engine did not answer the hello within ${helloTimeoutMs / 1000} s"
    ended = true
    outbox.close()
    writer?.cancel()
    // Whatever the writer had not taken (or everything, never attached) stays for the next wire
    while (true) {
      val left = outbox.tryReceive().getOrNull() ?: break
      synchronized(ledger) { ledger.leftover.add(left) }
    }
    synchronized(ledger) { ledger.endedAt = now() }
    _events.send(EngineEvent.Closed(failure))
    _events.close()
  }

  private suspend fun receive(line: String, scope: CoroutineScope) {
    val message = try {
      SidecarMsg.decode(line)
    } catch (e: Exception) {
      _events.send(EngineEvent.Dropped("${line.take(120)}: ${e.message}"))
      return
    }
    when (message) {
      is SidecarMsg.HelloOkMsg -> {
        if (writer != null || helloTimedOut) return
        write(ShellMsg.AttachView(viewId, initial = initial))
        writer = scope.launch {
          for (action in outbox) deliver(action)
        }
        _events.send(EngineEvent.Connected(message.ok))
      }
      is SidecarMsg.HelloReject -> {
        if (message.protocolVersion != hello.protocolVersion) _events.send(EngineEvent.Incompatible(message.protocolVersion))
        throw IllegalStateException("The engine rejected this app: ${message.reason}")
      }
      is SidecarMsg.HostMessage -> if (message.viewId == viewId) _events.send(EngineEvent.Host(message.message))
      is SidecarMsg.Platform -> {
        val request = message.request
        if (request is PlatformRequest.Unknown) {
          message.requestId?.let { write(ShellMsg.PlatformResponse(it, error = "${request.method} is not supported by the phone")) }
        } else {
          _events.send(EngineEvent.Platform(request))
        }
      }
      is SidecarMsg.Unknown -> {}
    }
  }

  /** Writes one queued action; once the wire ended it stays in the ledger instead */
  private fun deliver(action: WebviewMsg) {
    if (ended) {
      synchronized(ledger) { ledger.leftover.add(action) }
      return
    }
    if (action.clientId != null) synchronized(ledger) {
      ledger.written.addLast(now() to action)
      while (ledger.written.size > WRITTEN_KEPT) ledger.written.removeFirst()
    }
    write(ShellMsg.WebviewMessage(viewId, action))
  }

  private fun now() = System.nanoTime() / 1_000_000

  /** A failed write ends nothing by itself: the reader sees the wire close */
  private fun write(message: ShellMsg) {
    transport.send(WireJson.encodeToString(kotlinx.serialization.json.JsonObject.serializer(), message.toJson()))
  }

  companion object {
    /** The platform methods the app answers; hello's capabilities name exactly these */
    val capabilities = listOf("toast", "notify")
    const val HELLO_TIMEOUT_MS = 15_000L
    /** How long before a wire's end a written action may still have been lost with it */
    const val UNSURE_WINDOW_MS = 45_000L
    const val WRITTEN_KEPT = 64
  }
}
