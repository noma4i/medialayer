package com.noma4i.medialayer.ui

import android.content.Context
import android.graphics.Bitmap
import android.graphics.drawable.Animatable
import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.MutableState
import androidx.compose.runtime.State
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.runtime.produceState
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asAndroidBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.Constraints
import coil3.DrawableImage
import coil3.Extras
import coil3.compose.AsyncImage
import coil3.compose.AsyncImagePainter
import coil3.ImageLoader
import coil3.decode.BitmapFactoryDecoder
import coil3.gif.repeatCount
import coil3.request.ImageResult
import coil3.request.ImageRequest
import coil3.size.Scale
import com.noma4i.medialayer.MediaItem
import com.noma4i.medialayer.MediaLayer
import com.noma4i.medialayer.MediaSource
import com.noma4i.medialayer.ImageWork
import com.noma4i.medialayer.MediaState
import com.noma4i.medialayer.Placeholder
import com.noma4i.medialayer.PixelSize
import com.noma4i.medialayer.decodeBlurhash
import com.noma4i.medialayer.toItem
import java.nio.ByteBuffer
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.awaitCancellation
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.onStart
import kotlinx.coroutines.flow.onCompletion
import kotlinx.coroutines.flow.transformLatest
import kotlinx.coroutines.withContext

@Composable
fun MediaImage(
  source: MediaSource,
  modifier: Modifier = Modifier,
  contentScale: ContentScale = ContentScale.Crop,
  preview: MediaSource? = null,
  playbackRate: Double = 1.0,
  animation: ImageAnimation = ImageAnimation(),
  content: @Composable (MediaState) -> Unit = {}
) {
  if (currentEnvironmentOrNull() == null) {
    Box(modifier) { content(MediaState.Idle) }
    return
  }
  val screen = rememberScreenVisibility(source.url, LocalMediaEnvironment.current.stats)
  val item = source.toItem()
  val failure = remember { mutableStateOf<ScopedFailure?>(null) }
  val mount = claimImage(source, item, screen, failure)
  if (source.size != null) {
    Box(modifier.then(screen.modifier)) { ImageBody(source, preview, item, mount, failure, null, 0L, screen, contentScale, animation, playbackRate, content) }
    return
  }
  val density = LocalDensity.current.density
  BoxWithConstraints(modifier.then(screen.modifier)) {
    val placementPixels = if (constraints.hasBoundedWidth && constraints.hasBoundedHeight) constraints.maxWidth.toLong() * constraints.maxHeight else 0L
    ImageBody(source, preview, item, mount, failure, null, placementPixels, screen, contentScale, animation, playbackRate, content, decodeInput(constraints, density, source, contentScale))
  }
}

@Composable
private fun claimImage(source: MediaSource, item: MediaItem, screen: ScreenVisibility, failure: MutableState<ScopedFailure?>): PlaceMount {
  val environment = LocalMediaEnvironment.current
  val media = environment.media
  val stats = environment.stats
  val mount = remember(media, item) { PlaceMount(media, item) { failure.value = ScopedFailure(item, MediaState.Failed(it.toString())) } }
  DisposableEffect(mount) { onDispose { mount.close() } }
  LaunchedEffect(mount, screen) {
    withContext(Dispatchers.Default) {
      snapshotFlow { screen.visible }.onStart { stats.recordLater(ImageWork.Observe) }.onCompletion { stats.recordLater(ImageWork.Unobserve) }.collect { visible ->
        mount.update(placeInput(visible = visible))
      }
    }
  }
  return mount
}

@Composable
@OptIn(ExperimentalCoroutinesApi::class)
private fun ImageBody(source: MediaSource, preview: MediaSource?, item: MediaItem, mount: PlaceMount, failure: State<ScopedFailure?>, pixels: PixelSize?, placementPixels: Long, screen: ScreenVisibility, contentScale: ContentScale, animation: ImageAnimation, playbackRate: Double, content: @Composable (MediaState) -> Unit, dimensions: PixelSizeMemo.Args? = null) {
  val context = LocalContext.current
  val environment = LocalMediaEnvironment.current
  val media = environment.media
  val scale = contentScale.toScale()
  val stats = environment.stats
  val reduceMotion by SystemReduceMotion.state
  val finished = remember(source.url, animation.repeats, animation.playing) { mutableStateOf(false) }
  val rests = animation.rests(reduceMotion) || finished.value
  val resting = if (rests) animation.rest else null
  val rate = playbackRateOf(playbackRate)
  val rated = playsAtRate(rate, resting)
  val request = remember(media, source, pixels, scale, placementPixels, resting, animation.repeats, dimensions, rated) { imageRequest(context, source, pixels, scale, media, placementPixels, resting, animation.repeats, media.viewDecodeAdmit(), dimensions, rate) }
  val state = remember(item) { mutableStateOf<MediaState>(MediaState.Idle) }
  val animated = remember { mutableStateOf<Animatable?>(null) }
  LaunchedEffect(failure.value, item) { failure.value.of(item)?.let { state.value = it } }
  LaunchedEffect(animated.value, rate) { (animated.value as? RatedAnimation)?.rateTo(rate) }
  LaunchedEffect(screen) {
    snapshotFlow { animated.value to screen.visible }.onStart { stats.recordLater(ImageWork.Observe) }.onCompletion { stats.recordLater(ImageWork.Unobserve) }.collectLatest { (drawable, visible) ->
      if (visible) {
        drawable?.repeating()?.onFinished { finished.value = true }
        drawable?.start()
      } else {
        drawable?.stop()
      }
      try { awaitCancellation() } finally { drawable?.stop() }
    }
  }
  val loader = remember(environment.imageLoader, screen) {
    object : ImageLoader by environment.imageLoader {
      override suspend fun execute(request: ImageRequest): ImageResult =
        // Visibility cancels in-flight decoding without invalidating the image composition.
        snapshotFlow { screen.visible }.onStart { stats.recordLater(ImageWork.Observe) }.onCompletion { stats.recordLater(ImageWork.Unobserve) }.transformLatest { visible ->
          if (visible) {
            MediaTrace.emit("image") { "load ${screen.url.takeLast(24)} start" }
            val result = environment.imageLoader.execute(request)
            MediaTrace.emit("image") { "load ${screen.url.takeLast(24)} ${if (result is coil3.request.ErrorResult) "failed" else "done"}" }
            emit(result)
          }
        }.first()
    }
  }
  ImageBlur(item, screen, state, contentScale)
  if (preview != null && screen.visible && state.value != MediaState.Ready) {
    val previewRequest = remember(media, preview, pixels, scale, placementPixels, dimensions) { imageRequest(context, preview, pixels, scale, media, placementPixels, admit = media.viewDecodeAdmit(), dimensions = dimensions?.copy(declared = preview.size?.let { PixelSize(it.first.toUInt(), it.second.toUInt()) })) }
    AsyncImage(model = previewRequest, contentDescription = null, imageLoader = loader, contentScale = contentScale, modifier = Modifier.fillMaxSize().testTag(PREVIEW_TAG))
  }
  AsyncImage(
    model = request,
    contentDescription = null,
    imageLoader = loader,
    contentScale = contentScale,
    onState = { painterState ->
      animated.value = ((painterState as? AsyncImagePainter.State.Success)?.result?.image as? DrawableImage)?.drawable as? Animatable
      painterState.toMediaState()?.let { reached ->
        state.value = reached
        mount.report(reached)
      }
    },
    modifier = Modifier.fillMaxSize().drawWithContent { if (state.value.drawsImage()) drawContent() }
  )
  ImageContent(screen, state, content)
}

internal fun MediaState.drawsImage(): Boolean = this != MediaState.Idle && this != MediaState.Loading

@Composable
private fun ImageContent(screen: ScreenVisibility, state: State<MediaState>, content: @Composable (MediaState) -> Unit) {
  if (screen.visible) content(state.value)
}

@Composable
private fun ImageBlur(item: MediaItem, screen: ScreenVisibility, state: State<MediaState>, contentScale: ContentScale) {
  if (!screen.visible || state.value == MediaState.Ready) return
  val media = LocalMediaEnvironment.current.media
  val decoder = LocalBlurDecoder.current
  val observer = LocalPlaceholderObserver.current
  key(media, item) {
    val blur by produceState<ImageBitmap?>(null) {
      val answer = withContext(Dispatchers.Default) { media.placeholder(item, false) as? Placeholder.BlurHash }
      value = answer?.let { decodeBlur(it.hash, decoder) }
      value?.let { observer?.invoke("blurhash", it.asAndroidBitmap()) }
    }
    blur?.let { Image(it, "blurhash", Modifier.fillMaxSize(), contentScale = contentScale) }
  }
}

internal fun decodeInput(constraints: Constraints, density: Float, source: MediaSource, contentScale: ContentScale): PixelSizeMemo.Args {
  val declared = source.size?.let { PixelSize(it.first.toUInt(), it.second.toUInt()) }
  val width = if (constraints.hasBoundedWidth) constraints.maxWidth / density.toDouble() else 0.0
  val height = if (constraints.hasBoundedHeight) constraints.maxHeight / density.toDouble() else 0.0
  return PixelSizeMemo.Args(width, height, density.toDouble(), declared, contentScale.toScale() == Scale.FILL)
}

internal fun imageRequest(context: Context, source: MediaSource, pixels: PixelSize?, scale: Scale, media: MediaLayer, placementPixels: Long = 0, rest: RestFrame? = null, repeats: Repeats = Repeats.Forever, admit: DecodeAdmit, dimensions: PixelSizeMemo.Args? = null, rate: Double = 1.0): ImageRequest =
  ImageRequest.Builder(context)
    .data(MediaImageData(source.url, source.headers.toMap(), media, if (pixels == null) source.size?.let { PixelSize(it.first.toUInt(), it.second.toUInt()) } else null, pixels, rest, repeats, dimensions, playsAtRate(rate, rest)))
    .apply {
      when (rest) {
        RestFrame.First -> decoderFactory(AdmittedDecoders(admit, BitmapFactoryDecoder.Factory()))
        RestFrame.Last -> decoderFactory(RestFrameDecoder.Factory(admit))
        null -> if (playsAtRate(rate, null)) decoderFactory(RatedAnimationDecoder.Factory(admit, playbackRateOf(rate), repeats)) else (repeats as? Repeats.Times)?.let { repeatCount(maxOf(it.count, 1) - 1) }
      }
    }
    .fetcherCoroutineContext(Dispatchers.IO)
    .decoderCoroutineContext(Dispatchers.Default)
    .scale(scale)
    .apply {
      extras[PLACEMENT_PIXELS] = placementPixels
      pixels?.let {
        size(it.width.toInt(), it.height.toInt())
      }
    }
    .build()

internal const val PREVIEW_TAG = "medialayer-preview"

internal val PLACEMENT_PIXELS = Extras.Key(default = 0L)

internal fun ContentScale.toScale(): Scale = when (this) {
  ContentScale.Fit, ContentScale.Inside, ContentScale.FillWidth, ContentScale.FillHeight -> Scale.FIT
  else -> Scale.FILL
}

internal val LocalPlaceholderObserver = staticCompositionLocalOf<((String, Any) -> Unit)?> { null }

internal val LocalBlurDecoder = staticCompositionLocalOf<(String) -> ByteArray> { { hash -> decodeBlurhash(hash, BLUR_SIZE.toUInt(), BLUR_SIZE.toUInt(), 1f) } }

private suspend fun decodeBlur(hash: String, decode: (String) -> ByteArray): ImageBitmap? = withContext(Dispatchers.Default) {
  ensureActive()
  try {
    val pixels = decode(hash)
    Bitmap.createBitmap(BLUR_SIZE, BLUR_SIZE, Bitmap.Config.ARGB_8888).apply { copyPixelsFromBuffer(ByteBuffer.wrap(pixels)) }.asImageBitmap()
  } catch (error: CancellationException) {
    throw error
  } catch (error: Throwable) {
    null
  }
}

private const val BLUR_SIZE = 32

internal fun imageFailure(error: Throwable): MediaState? =
  if (error is CancellationException) null else MediaState.Failed(error.message ?: error.javaClass.simpleName)

private fun AsyncImagePainter.State.toMediaState(): MediaState? = when (this) {
  is AsyncImagePainter.State.Empty -> MediaState.Idle
  is AsyncImagePainter.State.Loading -> MediaState.Loading
  is AsyncImagePainter.State.Success -> MediaState.Ready
  is AsyncImagePainter.State.Error -> imageFailure(result.throwable)
}
