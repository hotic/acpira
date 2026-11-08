package cc.hotic.acpira.store

import android.content.Context
import android.graphics.SurfaceTexture
import android.media.MediaCodec
import android.media.MediaCodecInfo
import android.media.MediaExtractor
import android.media.MediaFormat
import android.media.MediaMetadataRetriever
import android.media.MediaMuxer
import android.net.Uri
import android.opengl.EGL14
import android.opengl.EGLConfig
import android.opengl.EGLContext
import android.opengl.EGLDisplay
import android.opengl.EGLExt
import android.opengl.EGLSurface
import android.opengl.GLES11Ext
import android.opengl.GLES20
import android.os.Handler
import android.os.HandlerThread
import android.os.SystemClock
import android.view.Surface
import java.io.File
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.nio.FloatBuffer
import kotlin.coroutines.coroutineContext
import kotlin.math.max
import kotlin.math.min
import kotlin.math.roundToInt
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/**
 * Re-encodes a picked video into an H.264 MP4 small enough for the paired link (apps/ios Chat/VideoCompressor.swift),
 * with the platform's codecs only: MediaCodec decodes into a SurfaceTexture, GL draws each frame into the encoder's
 * input surface at the target size, MediaMuxer writes the file. The long side, the bitrate and the frame rate step
 * down together as the video gets longer; a video that would need less than the floor bitrate is refused.
 *
 * Where it parts from iOS: an AAC audio track (what phones record) is copied as it is rather than re-encoded, and any
 * other audio is left out; the muxer cannot put the moov atom up front; an HDR clip is not tone-mapped.
 */
object VideoCompressor {
  sealed class Failure(message: String) : Exception(message) {
    class TooBig : Failure("too big")
    class Unreadable : Failure("unreadable")
  }

  /** The largest copy that goes (VideoCompressor.maxBytes): where chat apps put theirs, the relay uploads slowly */
  const val MAX_BYTES = 35 shl 20

  /** Long side and its full bitrate, best first; a step is taken while the budget affords its bitrate */
  private val LADDER = listOf(1280 to 2_500_000, 960 to 1_500_000, 640 to 800_000)
  /** Below this the picture falls apart: the video is too long for the budget */
  private const val FLOOR_BITRATE = 300_000
  /** The copied audio's rate when its track does not say */
  private const val AUDIO_BITRATE = 128_000
  private const val MAX_FRAME_RATE = 30
  private const val KEY_FRAME_SECONDS = 2
  /** How long the copy loop waits with nothing moving before it gives up (VideoCompressor.stallLimit) */
  private const val STALL_MS = 20_000L
  private const val DEQUEUE_US = 10_000L
  /** How long a rendered frame may take to reach the SurfaceTexture */
  private const val FRAME_WAIT_MS = 2_500L

  /** Writes [source] compressed to [out] (its folder is made), reporting 0…1 as it goes; throws [Failure] */
  suspend fun compress(context: Context, source: Uri, out: File, budget: Int, progress: (Float) -> Unit) = withContext(Dispatchers.Default) {
    out.parentFile?.mkdirs()
    out.delete()
    var done = false
    try {
      // Not suspended inside: the GL context stays current on this one thread throughout
      val job = coroutineContext[kotlinx.coroutines.Job]
      val active = { job?.isActive != false }
      // A decoder that fails before its first frame (a vendor codec the app cannot share buffers with, seen on an
      // emulator image) hands over to the next one that takes the clip
      val candidates = decoders(context, source)
      for ((i, name) in candidates.withIndex()) {
        val run = Transcode(context, source, out, budget, progress, active, name)
        try {
          run.run()
          break
        } catch (e: Exception) {
          if (e is Failure || e is kotlinx.coroutines.CancellationException || run.framesOut > 0 || i == candidates.lastIndex) throw e
          android.util.Log.w(TAG, "decoder $name failed, trying the next", e)
          out.delete()
        }
      }
      if (out.length() > budget) throw Failure.TooBig()
      progress(1f)
      done = true
    } catch (e: Failure) {
      throw e
    } catch (e: kotlinx.coroutines.CancellationException) {
      throw e
    } catch (e: Exception) {
      android.util.Log.w(TAG, "transcode failed", e)
      throw Failure.Unreadable()
    } finally {
      if (!done) out.delete()
    }
  }

  /** One transcode: every codec, surface and file it opens is released when [run] returns */
  private class Transcode(
    val context: Context, val source: Uri, val out: File, val budget: Int, val progress: (Float) -> Unit, val active: () -> Boolean,
    /** The decoder to use; null for the platform's choice */
    val decoderName: String?,
  ) {
    private val closers = ArrayList<() -> Unit>()
    /** Frames the decoder has produced: after the first, a failure is the clip's, not the decoder's */
    var framesOut = 0
      private set

    private fun <T> keep(value: T, close: (T) -> Unit): T = value.also { closers += { runCatching { close(it) } } }

    fun run() {
      try {
        transcode()
      } finally {
        closers.asReversed().forEach { it() }
      }
    }

    private fun extractor(): MediaExtractor = keep(MediaExtractor()) { it.release() }.also { it.setDataSource(context, source, null) }

    private fun track(e: MediaExtractor, prefix: String): Int =
      (0 until e.trackCount).firstOrNull { e.getTrackFormat(it).getString(MediaFormat.KEY_MIME)?.startsWith(prefix) == true } ?: -1

    private fun transcode() {
      val video = extractor()
      val videoTrack = track(video, "video/")
      if (videoTrack < 0) throw Failure.Unreadable()
      video.selectTrack(videoTrack)
      val format = video.getTrackFormat(videoTrack)
      val width = format.getInteger(MediaFormat.KEY_WIDTH)
      val height = format.getInteger(MediaFormat.KEY_HEIGHT)
      val durationUs = if (format.containsKey(MediaFormat.KEY_DURATION)) format.getLong(MediaFormat.KEY_DURATION) else 0L
      if (width <= 0 || height <= 0 || durationUs <= 0) throw Failure.Unreadable()
      val fps = if (format.containsKey(MediaFormat.KEY_FRAME_RATE)) runCatching { format.getInteger(MediaFormat.KEY_FRAME_RATE) }.getOrElse { format.getFloat(MediaFormat.KEY_FRAME_RATE).roundToInt() } else 0
      val rotation = rotationOf()

      // AAC is copied into the new file; any other audio is left out
      val audio = extractor()
      val audioTrack = track(audio, "audio/").takeIf { it >= 0 && audio.getTrackFormat(it).getString(MediaFormat.KEY_MIME) == MediaFormat.MIMETYPE_AUDIO_AAC }
      val audioFormat = audioTrack?.let { audio.selectTrack(it); audio.getTrackFormat(it) }
      val audioBitrate = audioFormat?.let { if (it.containsKey(MediaFormat.KEY_BIT_RATE)) it.getInteger(MediaFormat.KEY_BIT_RATE) else AUDIO_BITRATE } ?: 0

      // What the budget allows per second, less the audio and a tenth for the encoder's swings around its average
      val seconds = durationUs / 1_000_000.0
      val affordable = budget * 8 * 0.9 / seconds - audioBitrate
      if (affordable < FLOOR_BITRATE) throw Failure.TooBig()
      val step = LADDER.firstOrNull { it.second <= affordable } ?: LADDER.last()
      val bitrate = min(step.second.toDouble(), affordable).toInt()
      // Never upscaled; even sides, as H.264 wants
      val scale = min(1.0, step.first.toDouble() / max(width, height))
      val outW = max(2, (width * scale / 2).roundToInt() * 2)
      val outH = max(2, (height * scale / 2).roundToInt() * 2)
      val frameRate = min(if (fps > 0) fps else MAX_FRAME_RATE, MAX_FRAME_RATE)

      val encoderFormat = MediaFormat.createVideoFormat(MediaFormat.MIMETYPE_VIDEO_AVC, outW, outH).apply {
        setInteger(MediaFormat.KEY_COLOR_FORMAT, MediaCodecInfo.CodecCapabilities.COLOR_FormatSurface)
        setInteger(MediaFormat.KEY_BIT_RATE, bitrate)
        setInteger(MediaFormat.KEY_FRAME_RATE, frameRate)
        setInteger(MediaFormat.KEY_I_FRAME_INTERVAL, KEY_FRAME_SECONDS)
      }
      val encoder = keep(MediaCodec.createEncoderByType(MediaFormat.MIMETYPE_VIDEO_AVC)) { it.release() }
      encoder.configure(encoderFormat, null, null, MediaCodec.CONFIGURE_FLAG_ENCODE)
      val egl = keep(EncoderSurface(encoder.createInputSurface())) { it.release() }
      val frames = keep(FrameSink()) { it.release() }
      encoder.start()
      keep(encoder) { it.stop() }

      // The decoder draws upright frames itself when told the rotation; the muxer's hint carries it instead
      format.setInteger(MediaFormat.KEY_ROTATION, 0)
      val decoder = keep(decoderName?.let { MediaCodec.createByCodecName(it) } ?: MediaCodec.createDecoderByType(format.getString(MediaFormat.KEY_MIME)!!)) { it.release() }
      decoder.configure(format, frames.surface, null, 0)
      decoder.start()
      keep(decoder) { it.stop() }

      val muxer = keep(MediaMuxer(out.path, MediaMuxer.OutputFormat.MUXER_OUTPUT_MPEG_4)) { it.release() }
      muxer.setOrientationHint(rotation)
      var muxerVideo = -1
      var muxerAudio = -1
      var started = false
      val audioBuffer = ByteBuffer.allocateDirect(audioFormat?.takeIf { it.containsKey(MediaFormat.KEY_MAX_INPUT_SIZE) }?.getInteger(MediaFormat.KEY_MAX_INPUT_SIZE) ?: AUDIO_SAMPLE_BYTES)
      val audioInfo = MediaCodec.BufferInfo()
      var audioDone = audioFormat == null

      /** Copies the audio up to [untilUs] (all of it for Long.MAX_VALUE), so the two tracks interleave */
      fun pumpAudio(untilUs: Long) {
        while (!audioDone && started) {
          if (audio.sampleTrackIndex < 0) { audioDone = true; break }
          val at = audio.sampleTime
          if (at > untilUs) break
          // AAC's priming frame comes before zero (the encoder delay), which the muxer refuses
          if (at < 0) { audio.advance(); continue }
          audioInfo.size = audio.readSampleData(audioBuffer, 0)
          if (audioInfo.size < 0) { audioDone = true; break }
          audioInfo.offset = 0
          audioInfo.presentationTimeUs = at
          audioInfo.flags = if (audio.sampleFlags and MediaExtractor.SAMPLE_FLAG_SYNC != 0) MediaCodec.BUFFER_FLAG_KEY_FRAME else 0
          muxer.writeSampleData(muxerAudio, audioBuffer, audioInfo)
          audio.advance()
        }
      }

      val info = MediaCodec.BufferInfo()
      var inputDone = false
      var decoderDone = false
      var encoderDone = false
      // A clip shot above 30 fps drops frames down to it; frame times keep their spacing, so playback speed holds
      val minGapUs = 1_000_000L / MAX_FRAME_RATE - FRAME_SLACK_US
      var lastKept = Long.MIN_VALUE / 2
      var lastMoved = SystemClock.elapsedRealtime()
      while (!encoderDone) {
        if (!active()) throw kotlinx.coroutines.CancellationException()
        var moved = false
        if (!inputDone) {
          val index = decoder.dequeueInputBuffer(DEQUEUE_US)
          if (index >= 0) {
            moved = true
            val buffer = decoder.getInputBuffer(index)!!
            val size = video.readSampleData(buffer, 0)
            if (size < 0) {
              decoder.queueInputBuffer(index, 0, 0, 0, MediaCodec.BUFFER_FLAG_END_OF_STREAM)
              inputDone = true
            } else {
              decoder.queueInputBuffer(index, 0, size, video.sampleTime, 0)
              video.advance()
            }
          }
        }
        if (!decoderDone) {
          val index = decoder.dequeueOutputBuffer(info, DEQUEUE_US)
          if (index >= 0) {
            moved = true
            val pts = info.presentationTimeUs
            val render = info.size > 0 && pts - lastKept >= minGapUs
            decoder.releaseOutputBuffer(index, render)
            if (render) {
              if (!frames.await(FRAME_WAIT_MS)) throw Failure.Unreadable()
              frames.draw(outW, outH)
              egl.present(pts * 1000)
              lastKept = pts
              framesOut++
              progress((pts.toDouble() / durationUs).toFloat().coerceIn(0f, 1f))
            }
            if (info.flags and MediaCodec.BUFFER_FLAG_END_OF_STREAM != 0) {
              encoder.signalEndOfInputStream()
              decoderDone = true
            }
          }
        }
        while (true) {
          val index = encoder.dequeueOutputBuffer(info, if (decoderDone) DEQUEUE_US else 0)
          if (index == MediaCodec.INFO_TRY_AGAIN_LATER) break
          if (index == MediaCodec.INFO_OUTPUT_FORMAT_CHANGED) {
            muxerVideo = muxer.addTrack(encoder.outputFormat)
            if (audioFormat != null) muxerAudio = muxer.addTrack(audioFormat)
            muxer.start()
            started = true
            continue
          }
          if (index < 0) continue
          moved = true
          val buffer = encoder.getOutputBuffer(index)!!
          if (info.flags and MediaCodec.BUFFER_FLAG_CODEC_CONFIG != 0) info.size = 0
          if (info.size > 0 && started) {
            buffer.position(info.offset).limit(info.offset + info.size)
            muxer.writeSampleData(muxerVideo, buffer, info)
            pumpAudio(info.presentationTimeUs)
          }
          encoder.releaseOutputBuffer(index, false)
          if (info.flags and MediaCodec.BUFFER_FLAG_END_OF_STREAM != 0) { encoderDone = true; break }
        }
        if (moved) lastMoved = SystemClock.elapsedRealtime()
        // An encoder can stall without failing (a VM without a hardware encoder): it would spin here for good
        else if (SystemClock.elapsedRealtime() - lastMoved > STALL_MS) throw Failure.Unreadable()
      }
      if (!started) throw Failure.Unreadable()
      pumpAudio(Long.MAX_VALUE)
      muxer.stop()
    }

    /** The clip's rotation as recorded (a portrait phone clip keeps landscape frames and says 90) */
    private fun rotationOf(): Int {
      val retriever = MediaMetadataRetriever()
      return try {
        retriever.setDataSource(context, source)
        retriever.extractMetadata(MediaMetadataRetriever.METADATA_KEY_VIDEO_ROTATION)?.toIntOrNull() ?: 0
      } catch (_: Exception) {
        0
      } finally {
        retriever.release()
      }
    }
  }

  /** Every decoder for the clip's video track, the platform's pick first; just null (the platform's choice) when the clip cannot be read here */
  private fun decoders(context: Context, source: Uri): List<String?> {
    val format = runCatching {
      val e = MediaExtractor()
      try {
        e.setDataSource(context, source, null)
        (0 until e.trackCount).map { e.getTrackFormat(it) }.firstOrNull { it.getString(MediaFormat.KEY_MIME)?.startsWith("video/") == true }
      } finally {
        e.release()
      }
    }.getOrNull() ?: return listOf(null)
    val list = android.media.MediaCodecList(android.media.MediaCodecList.REGULAR_CODECS)
    val mime = format.getString(MediaFormat.KEY_MIME)
    val all = list.codecInfos.filter { !it.isEncoder && it.supportedTypes.any { t -> t.equals(mime, ignoreCase = true) } }.map { it.name }
    val first = runCatching { list.findDecoderForFormat(format) }.getOrNull()
    return (listOfNotNull(first) + all).distinct().ifEmpty { listOf(null) }
  }

  /** The encoder's input surface as an EGL window, current on the transcoding thread */
  private class EncoderSurface(private val surface: Surface) {
    private val display: EGLDisplay = EGL14.eglGetDisplay(EGL14.EGL_DEFAULT_DISPLAY)
    private val context: EGLContext
    private val window: EGLSurface

    init {
      val version = IntArray(2)
      check(EGL14.eglInitialize(display, version, 0, version, 1))
      val attribs = intArrayOf(
        EGL14.EGL_RED_SIZE, 8, EGL14.EGL_GREEN_SIZE, 8, EGL14.EGL_BLUE_SIZE, 8,
        EGL14.EGL_RENDERABLE_TYPE, EGL14.EGL_OPENGL_ES2_BIT, EGL_RECORDABLE_ANDROID, 1, EGL14.EGL_NONE,
      )
      val configs = arrayOfNulls<EGLConfig>(1)
      val count = IntArray(1)
      check(EGL14.eglChooseConfig(display, attribs, 0, configs, 0, 1, count, 0) && count[0] > 0)
      context = EGL14.eglCreateContext(display, configs[0], EGL14.EGL_NO_CONTEXT, intArrayOf(EGL14.EGL_CONTEXT_CLIENT_VERSION, 2, EGL14.EGL_NONE), 0)
      check(context != EGL14.EGL_NO_CONTEXT)
      window = EGL14.eglCreateWindowSurface(display, configs[0], surface, intArrayOf(EGL14.EGL_NONE), 0)
      check(window != EGL14.EGL_NO_SURFACE)
      check(EGL14.eglMakeCurrent(display, window, window, context))
    }

    /** Hands the drawn frame to the encoder, stamped with its time */
    fun present(nanos: Long) {
      EGLExt.eglPresentationTimeANDROID(display, window, nanos)
      EGL14.eglSwapBuffers(display, window)
    }

    fun release() {
      EGL14.eglMakeCurrent(display, EGL14.EGL_NO_SURFACE, EGL14.EGL_NO_SURFACE, EGL14.EGL_NO_CONTEXT)
      EGL14.eglDestroySurface(display, window)
      EGL14.eglDestroyContext(display, context)
      EGL14.eglReleaseThread()
      EGL14.eglTerminate(display)
      surface.release()
    }
  }

  /**
   * The decoder's output: a SurfaceTexture on an external texture in the current GL context, and the program that draws
   * it across the frame. Its frame callback arrives on a thread of its own, since the transcoding thread has no looper
   */
  private class FrameSink : SurfaceTexture.OnFrameAvailableListener {
    private val thread = HandlerThread("video-frames").apply { start() }
    private val lock = Object()
    private var ready = false
    private val texture: Int
    private val program: Int
    private val texture2: SurfaceTexture
    val surface: Surface
    private val matrix = FloatArray(16)
    private val quad: FloatBuffer = ByteBuffer.allocateDirect(QUAD.size * 4).order(ByteOrder.nativeOrder()).asFloatBuffer().put(QUAD).apply { position(0) }

    init {
      val ids = IntArray(1)
      GLES20.glGenTextures(1, ids, 0)
      texture = ids[0]
      GLES20.glBindTexture(GLES11Ext.GL_TEXTURE_EXTERNAL_OES, texture)
      GLES20.glTexParameteri(GLES11Ext.GL_TEXTURE_EXTERNAL_OES, GLES20.GL_TEXTURE_MIN_FILTER, GLES20.GL_LINEAR)
      GLES20.glTexParameteri(GLES11Ext.GL_TEXTURE_EXTERNAL_OES, GLES20.GL_TEXTURE_MAG_FILTER, GLES20.GL_LINEAR)
      GLES20.glTexParameteri(GLES11Ext.GL_TEXTURE_EXTERNAL_OES, GLES20.GL_TEXTURE_WRAP_S, GLES20.GL_CLAMP_TO_EDGE)
      GLES20.glTexParameteri(GLES11Ext.GL_TEXTURE_EXTERNAL_OES, GLES20.GL_TEXTURE_WRAP_T, GLES20.GL_CLAMP_TO_EDGE)
      program = link(VERTEX, FRAGMENT)
      texture2 = SurfaceTexture(texture)
      texture2.setOnFrameAvailableListener(this, Handler(thread.looper))
      surface = Surface(texture2)
    }

    override fun onFrameAvailable(st: SurfaceTexture) {
      synchronized(lock) {
        ready = true
        lock.notifyAll()
      }
    }

    /** Waits for the frame just released to the surface; false when it never came */
    fun await(timeoutMs: Long): Boolean {
      val until = SystemClock.elapsedRealtime() + timeoutMs
      synchronized(lock) {
        while (!ready) {
          val left = until - SystemClock.elapsedRealtime()
          if (left <= 0) return false
          lock.wait(left)
        }
        ready = false
      }
      return true
    }

    /** Latches the frame and draws it across a [width] x [height] target */
    fun draw(width: Int, height: Int) {
      texture2.updateTexImage()
      texture2.getTransformMatrix(matrix)
      GLES20.glViewport(0, 0, width, height)
      GLES20.glUseProgram(program)
      GLES20.glActiveTexture(GLES20.GL_TEXTURE0)
      GLES20.glBindTexture(GLES11Ext.GL_TEXTURE_EXTERNAL_OES, texture)
      val position = GLES20.glGetAttribLocation(program, "aPosition")
      val coord = GLES20.glGetAttribLocation(program, "aCoord")
      quad.position(0)
      GLES20.glVertexAttribPointer(position, 2, GLES20.GL_FLOAT, false, QUAD_STRIDE, quad)
      GLES20.glEnableVertexAttribArray(position)
      quad.position(2)
      GLES20.glVertexAttribPointer(coord, 2, GLES20.GL_FLOAT, false, QUAD_STRIDE, quad)
      GLES20.glEnableVertexAttribArray(coord)
      GLES20.glUniformMatrix4fv(GLES20.glGetUniformLocation(program, "uST"), 1, false, matrix, 0)
      GLES20.glDrawArrays(GLES20.GL_TRIANGLE_STRIP, 0, 4)
    }

    fun release() {
      surface.release()
      texture2.release()
      GLES20.glDeleteProgram(program)
      GLES20.glDeleteTextures(1, intArrayOf(texture), 0)
      thread.quitSafely()
    }

    private fun link(vertex: String, fragment: String): Int {
      fun shader(type: Int, source: String) = GLES20.glCreateShader(type).also {
        GLES20.glShaderSource(it, source)
        GLES20.glCompileShader(it)
      }
      return GLES20.glCreateProgram().also {
        GLES20.glAttachShader(it, shader(GLES20.GL_VERTEX_SHADER, vertex))
        GLES20.glAttachShader(it, shader(GLES20.GL_FRAGMENT_SHADER, fragment))
        GLES20.glLinkProgram(it)
        val ok = IntArray(1)
        GLES20.glGetProgramiv(it, GLES20.GL_LINK_STATUS, ok, 0)
        check(ok[0] == GLES20.GL_TRUE)
      }
    }
  }

  private const val TAG = "VideoCompressor"
  private const val EGL_RECORDABLE_ANDROID = 0x3142
  /** A frame time within this of the 30 fps spacing still counts as spaced (VideoCompressor's 2 ms) */
  private const val FRAME_SLACK_US = 2_000L
  private const val AUDIO_SAMPLE_BYTES = 256 * 1024
  /** x, y, s, t per corner, as a triangle strip */
  private val QUAD = floatArrayOf(-1f, -1f, 0f, 0f, 1f, -1f, 1f, 0f, -1f, 1f, 0f, 1f, 1f, 1f, 1f, 1f)
  private const val QUAD_STRIDE = 4 * 4
  private const val VERTEX = """
    attribute vec4 aPosition;
    attribute vec4 aCoord;
    uniform mat4 uST;
    varying vec2 vCoord;
    void main() { gl_Position = aPosition; vCoord = (uST * aCoord).xy; }
  """
  private const val FRAGMENT = """
    #extension GL_OES_EGL_image_external : require
    precision mediump float;
    varying vec2 vCoord;
    uniform samplerExternalOES sTexture;
    void main() { gl_FragColor = texture2D(sTexture, vCoord); }
  """
}
