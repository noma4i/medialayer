package com.noma4i.medialayer.ui

import android.hardware.camera2.CameraMetadata
import android.graphics.SurfaceTexture
import android.opengl.EGL14
import android.opengl.EGLConfig
import android.opengl.EGLContext
import android.opengl.EGLDisplay
import android.opengl.EGLExt
import android.opengl.EGLSurface
import android.opengl.GLES11Ext
import android.opengl.GLES20
import android.opengl.Matrix
import android.os.Handler
import android.os.HandlerThread
import android.util.Size
import android.view.Surface
import androidx.camera.core.CameraEffect
import androidx.camera.core.SurfaceOutput
import androidx.camera.core.SurfaceProcessor
import androidx.camera.core.SurfaceRequest
import androidx.core.util.Consumer
import com.noma4i.medialayer.FailureSource
import com.noma4i.medialayer.NativeFailure
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.nio.FloatBuffer
import java.util.concurrent.Executor

internal fun videoTimebaseOf(timestampSource: Int?): VideoTimebase =
  if (timestampSource == CameraMetadata.SENSOR_INFO_TIMESTAMP_SOURCE_REALTIME) VideoTimebase.BOOTTIME else VideoTimebase.UNKNOWN

internal class CameraVideoOutput(
  private val timebase: VideoTimebase,
  private val admit: (Long) -> AutoCloseable,
  private val onEncoderFailed: (Long, NativeFailure) -> Unit,
  private val onError: (Throwable) -> Unit
) {
  private class Preview(val output: SurfaceOutput, val target: Surface, var surface: EGLSurface = EGL14.EGL_NO_SURFACE)
  private class Encoder(val generation: Long, val target: Surface, val size: Size, val vertex: FloatArray, val limit: Int, var surface: EGLSurface = EGL14.EGL_NO_SURFACE) {
    val inFlight = ArrayDeque<Long>()
  }
  private class Input(val texture: Int, val st: SurfaceTexture, val surface: Surface, val lease: AutoCloseable, var lastTimestamp: Long = -1L, var released: Boolean = false)

  private val thread = HandlerThread("CameraVideoOutput-GL").apply { start() }
  private val handler = Handler(thread.looper)
  private val executor = Executor { handler.post(it) }
  private var display: EGLDisplay = EGL14.EGL_NO_DISPLAY
  private var context: EGLContext = EGL14.EGL_NO_CONTEXT
  private var idle: EGLSurface = EGL14.EGL_NO_SURFACE
  private var config: EGLConfig? = null
  private var program = 0
  private val previews = ArrayList<Preview>()
  private val inputs = ArrayList<Input>()
  private var current: Input? = null
  private var encoder: Encoder? = null
  private var newestGeneration = 0L
  private var shutting = false
  private var finished = false
  private var setupLease: AutoCloseable? = null
  private val texMatrix = FloatArray(16)
  private val outMatrix = FloatArray(16)
  private val identity = FloatArray(16).also { Matrix.setIdentityM(it, 0) }
  private val positions = floats(-1f, -1f, 1f, -1f, -1f, 1f, 1f, 1f)
  private val coordinates = floats(0f, 0f, 1f, 0f, 0f, 1f, 1f, 1f)

  private val processor = object : SurfaceProcessor {
    override fun onInputSurface(request: SurfaceRequest) {
      if (!handler.post { if (!shutting) attachInput(request) else request.willNotProvideSurface() }) request.willNotProvideSurface()
    }

    override fun onOutputSurface(output: SurfaceOutput) {
      if (!handler.post { if (!shutting) attachPreview(output) else output.close() }) output.close()
    }
  }

  val effect: CameraEffect = object : CameraEffect(CameraEffect.PREVIEW, executor, processor, Consumer<Throwable> { onError(it) }) {}

  init {
    handler.post { try { setup() } catch (error: Throwable) { shutdown(error) } }
  }

  fun attach(generation: Long, target: Surface, size: Size, vertexMatrix: FloatArray, inFlightLimit: Int) {
    handler.post {
      if (shutting || generation < newestGeneration) return@post
      newestGeneration = generation
      dropEncoder()
      if (timebase == VideoTimebase.UNKNOWN) return@post onEncoderFailed(generation, failure("camera timestamp source is not REALTIME"))
      if (inFlightLimit <= 0) return@post onEncoderFailed(generation, failure("no in-flight frame credit"))
      encoder = Encoder(generation, target, size, vertexMatrix.copyOf(), inFlightLimit)
    }
  }

  fun detach(generation: Long) {
    handler.post { if (encoder?.generation == generation) dropEncoder() }
  }

  fun encoded(generation: Long, ptsUs: Long) {
    handler.post {
      val target = encoder?.takeIf { it.generation == generation } ?: return@post
      while (target.inFlight.isNotEmpty() && target.inFlight.first() <= ptsUs) target.inFlight.removeFirst()
    }
  }

  fun release() {
    handler.post { shutdown(null) }
  }

  private fun shutdown(cause: Throwable?) {
    if (shutting) return
    shutting = true
    cause?.let(onError)
    encoder?.let { if (cause != null) onEncoderFailed(it.generation, failure(cause.message ?: cause.javaClass.name)) }
    dropEncoder()
    previews.toList().forEach { closePreview(it) }
    inputs.forEach { it.st.setOnFrameAvailableListener(null) }
    finishIfDone()
  }

  private fun finishIfDone() {
    if (!shutting || inputs.isNotEmpty() || finished) return
    finished = true
    step { if (context != EGL14.EGL_NO_CONTEXT) EGL14.eglMakeCurrent(display, idle, idle, context) }
    step { if (program != 0) GLES20.glDeleteProgram(program) }
    step { EGL14.eglMakeCurrent(display, EGL14.EGL_NO_SURFACE, EGL14.EGL_NO_SURFACE, EGL14.EGL_NO_CONTEXT) }
    step { if (idle != EGL14.EGL_NO_SURFACE) EGL14.eglDestroySurface(display, idle) }
    step { if (context != EGL14.EGL_NO_CONTEXT) EGL14.eglDestroyContext(display, context) }
    step { if (display != EGL14.EGL_NO_DISPLAY) EGL14.eglTerminate(display) }
    step { setupLease?.close() }
    thread.quitSafely()
  }

  private fun step(action: () -> Unit) {
    try { action() } catch (_: Throwable) {}
  }

  private fun setup() {
    setupLease = admit(PBUFFER_BYTES)
    display = EGL14.eglGetDisplay(EGL14.EGL_DEFAULT_DISPLAY)
    val version = IntArray(2)
    check(EGL14.eglInitialize(display, version, 0, version, 1)) { "eglInitialize failed ${EGL14.eglGetError()}" }
    val attributes = intArrayOf(
      EGL14.EGL_RENDERABLE_TYPE, EGL14.EGL_OPENGL_ES2_BIT, EGL14.EGL_SURFACE_TYPE, EGL14.EGL_WINDOW_BIT or EGL14.EGL_PBUFFER_BIT,
      EGL14.EGL_RED_SIZE, 8, EGL14.EGL_GREEN_SIZE, 8, EGL14.EGL_BLUE_SIZE, 8, EGL14.EGL_ALPHA_SIZE, 8, EGL_RECORDABLE_ANDROID, 1, EGL14.EGL_NONE
    )
    val configs = arrayOfNulls<EGLConfig>(1)
    val count = IntArray(1)
    check(EGL14.eglChooseConfig(display, attributes, 0, configs, 0, 1, count, 0) && count[0] > 0) { "no recordable EGL config" }
    config = configs[0]
    context = EGL14.eglCreateContext(display, config, EGL14.EGL_NO_CONTEXT, intArrayOf(EGL14.EGL_CONTEXT_CLIENT_VERSION, 2, EGL14.EGL_NONE), 0)
    check(context != EGL14.EGL_NO_CONTEXT) { "eglCreateContext failed ${EGL14.eglGetError()}" }
    idle = EGL14.eglCreatePbufferSurface(display, config, intArrayOf(EGL14.EGL_WIDTH, 1, EGL14.EGL_HEIGHT, 1, EGL14.EGL_NONE), 0)
    check(idle != EGL14.EGL_NO_SURFACE) { "pbuffer failed ${EGL14.eglGetError()}" }
    makeCurrent(idle)
    program = buildProgram()
  }

  private fun attachInput(request: SurfaceRequest) {
    var lease: AutoCloseable? = null
    var texture = 0
    var st: SurfaceTexture? = null
    try {
      lease = admit(request.resolution.width.toLong() * request.resolution.height * BYTES_PER_PIXEL)
      val ids = IntArray(1)
      GLES20.glGenTextures(1, ids, 0)
      texture = ids[0]
      GLES20.glBindTexture(GLES11Ext.GL_TEXTURE_EXTERNAL_OES, texture)
      GLES20.glTexParameteri(GLES11Ext.GL_TEXTURE_EXTERNAL_OES, GLES20.GL_TEXTURE_MIN_FILTER, GLES20.GL_LINEAR)
      GLES20.glTexParameteri(GLES11Ext.GL_TEXTURE_EXTERNAL_OES, GLES20.GL_TEXTURE_MAG_FILTER, GLES20.GL_LINEAR)
      st = SurfaceTexture(texture).apply { setDefaultBufferSize(request.resolution.width, request.resolution.height) }
      val made = Input(texture, st, Surface(st), lease)
      inputs.add(made)
      current = made
      st.setOnFrameAvailableListener({ draw(made) }, handler)
      request.provideSurface(made.surface, executor) { releaseInput(made) }
    } catch (error: Throwable) {
      val registered = inputs.firstOrNull { it.st === st }
      if (registered != null) {
        releaseInput(registered)
      } else {
        st?.release()
        if (texture != 0) step { GLES20.glDeleteTextures(1, intArrayOf(texture), 0) }
        step { lease?.close() }
        request.willNotProvideSurface()
      }
      shutdown(error)
    }
  }

  private fun releaseInput(made: Input) {
    if (made.released) return
    made.released = true
    inputs.remove(made)
    if (current === made) current = null
    made.st.setOnFrameAvailableListener(null)
    made.surface.release()
    made.st.release()
    step { GLES20.glDeleteTextures(1, intArrayOf(made.texture), 0) }
    step { made.lease.close() }
    finishIfDone()
  }

  private fun attachPreview(output: SurfaceOutput) {
    lateinit var preview: Preview
    val target = output.getSurface(executor) { event -> if (event.eventCode == SurfaceOutput.Event.EVENT_REQUEST_CLOSE) handler.post { closePreview(preview) } }
    preview = Preview(output, target)
    previews.add(preview)
  }

  private fun closePreview(preview: Preview) {
    if (!previews.remove(preview)) return
    step {
      if (preview.surface != EGL14.EGL_NO_SURFACE && !finished) {
        makeCurrent(idle)
        EGL14.eglDestroySurface(display, preview.surface)
      }
    }
    preview.output.close()
  }

  private fun dropEncoder() {
    val old = encoder ?: return
    encoder = null
    step {
      if (old.surface != EGL14.EGL_NO_SURFACE && !finished) {
        makeCurrent(idle)
        EGL14.eglDestroySurface(display, old.surface)
      }
    }
  }

  private fun draw(frame: Input) {
    if (frame.released || finished) return
    try {
      frame.st.updateTexImage()
      val timestamp = frame.st.timestamp
      if (frame !== current || shutting || timestamp == frame.lastTimestamp) return
      frame.lastTimestamp = timestamp
      frame.st.getTransformMatrix(texMatrix)
      previews.toList().forEach { preview ->
        if (preview.surface == EGL14.EGL_NO_SURFACE) preview.surface = windowSurface(preview.target)
        preview.output.updateTransformMatrix(outMatrix, texMatrix)
        render(preview.surface, preview.output.size, identity, outMatrix, frame.texture)
        check(EGL14.eglSwapBuffers(display, preview.surface)) { "preview swap failed ${EGL14.eglGetError()}" }
      }
      encoder?.let { drawEncoder(it, frame, timestamp) }
    } catch (error: Throwable) {
      shutdown(error)
    }
  }

  private fun drawEncoder(target: Encoder, frame: Input, timestamp: Long) {
    if (target.inFlight.size >= target.limit) return
    try {
      if (target.surface == EGL14.EGL_NO_SURFACE) target.surface = windowSurface(target.target)
      render(target.surface, target.size, target.vertex, texMatrix, frame.texture)
      EGLExt.eglPresentationTimeANDROID(display, target.surface, timestamp)
      check(EGL14.eglSwapBuffers(display, target.surface)) { "encoder swap failed ${EGL14.eglGetError()}" }
      target.inFlight.addLast(timestamp / 1000)
    } catch (error: Throwable) {
      dropEncoder()
      onEncoderFailed(target.generation, failure(error.message ?: error.javaClass.name))
    }
  }

  private fun render(surface: EGLSurface, size: Size, vertex: FloatArray, texture: FloatArray, sampler: Int) {
    makeCurrent(surface)
    GLES20.glViewport(0, 0, size.width, size.height)
    GLES20.glUseProgram(program)
    GLES20.glActiveTexture(GLES20.GL_TEXTURE0)
    GLES20.glBindTexture(GLES11Ext.GL_TEXTURE_EXTERNAL_OES, sampler)
    GLES20.glUniformMatrix4fv(GLES20.glGetUniformLocation(program, "uVertex"), 1, false, vertex, 0)
    GLES20.glUniformMatrix4fv(GLES20.glGetUniformLocation(program, "uTex"), 1, false, texture, 0)
    val position = GLES20.glGetAttribLocation(program, "aPos")
    val coordinate = GLES20.glGetAttribLocation(program, "aTex")
    GLES20.glEnableVertexAttribArray(position)
    GLES20.glVertexAttribPointer(position, 2, GLES20.GL_FLOAT, false, 0, positions)
    GLES20.glEnableVertexAttribArray(coordinate)
    GLES20.glVertexAttribPointer(coordinate, 2, GLES20.GL_FLOAT, false, 0, coordinates)
    GLES20.glDrawArrays(GLES20.GL_TRIANGLE_STRIP, 0, 4)
    check(GLES20.glGetError() == GLES20.GL_NO_ERROR) { "GL draw failed" }
  }

  private fun makeCurrent(surface: EGLSurface) = check(EGL14.eglMakeCurrent(display, surface, surface, context)) { "eglMakeCurrent failed ${EGL14.eglGetError()}" }

  private fun windowSurface(target: Surface): EGLSurface =
    EGL14.eglCreateWindowSurface(display, config, target, intArrayOf(EGL14.EGL_NONE), 0).also { check(it != EGL14.EGL_NO_SURFACE) { "window surface failed ${EGL14.eglGetError()}" } }

  private fun failure(message: String) = NativeFailure(FailureSource.RECORDER, "CameraVideoOutput", 0, message)

  private fun buildProgram(): Int {
    val vertex = compile(GLES20.GL_VERTEX_SHADER, "uniform mat4 uVertex; uniform mat4 uTex; attribute vec4 aPos; attribute vec4 aTex; varying vec2 vTex; void main() { gl_Position = uVertex * aPos; vTex = (uTex * aTex).xy; }")
    val fragment = compile(GLES20.GL_FRAGMENT_SHADER, "#extension GL_OES_EGL_image_external : require\nprecision mediump float; varying vec2 vTex; uniform samplerExternalOES uSampler; void main() { gl_FragColor = texture2D(uSampler, vTex); }")
    val made = GLES20.glCreateProgram()
    GLES20.glAttachShader(made, vertex)
    GLES20.glAttachShader(made, fragment)
    GLES20.glLinkProgram(made)
    GLES20.glDeleteShader(vertex)
    GLES20.glDeleteShader(fragment)
    val linked = IntArray(1)
    GLES20.glGetProgramiv(made, GLES20.GL_LINK_STATUS, linked, 0)
    if (linked[0] != GLES20.GL_TRUE) {
      val log = GLES20.glGetProgramInfoLog(made)
      GLES20.glDeleteProgram(made)
      error("GL link failed: $log")
    }
    return made
  }

  private fun compile(type: Int, source: String): Int {
    val shader = GLES20.glCreateShader(type)
    GLES20.glShaderSource(shader, source)
    GLES20.glCompileShader(shader)
    val status = IntArray(1)
    GLES20.glGetShaderiv(shader, GLES20.GL_COMPILE_STATUS, status, 0)
    if (status[0] != GLES20.GL_TRUE) {
      val log = GLES20.glGetShaderInfoLog(shader)
      GLES20.glDeleteShader(shader)
      error("GL compile failed: $log")
    }
    return shader
  }

  private companion object {
    const val EGL_RECORDABLE_ANDROID = 0x3142
    const val BYTES_PER_PIXEL = 4L
    const val PBUFFER_BYTES = BYTES_PER_PIXEL

    fun floats(vararg values: Float): FloatBuffer =
      ByteBuffer.allocateDirect(values.size * 4).order(ByteOrder.nativeOrder()).asFloatBuffer().apply { put(values); position(0) }
  }
}
