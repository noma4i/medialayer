package com.noma4i.medialayer.ui

import android.content.Context
import android.graphics.Bitmap
import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.produceState
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.Constraints
import coil.compose.AsyncImage
import coil.compose.AsyncImagePainter
import coil.memory.MemoryCache
import coil.request.ImageRequest
import coil.size.Scale
import com.noma4i.medialayer.MediaSource
import com.noma4i.medialayer.MediaState
import com.noma4i.medialayer.Placeholder
import com.noma4i.medialayer.PixelSize
import com.noma4i.medialayer.decodeBlurhash
import com.noma4i.medialayer.imagePixelSize
import com.noma4i.medialayer.toItem
import java.nio.ByteBuffer
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

@Composable
fun MediaImage(
  source: MediaSource,
  modifier: Modifier = Modifier,
  contentScale: ContentScale = ContentScale.Crop,
  weight: MediaWeight = MediaWeight.Full,
  content: @Composable (MediaState) -> Unit = {}
) {
  when (weight) {
    MediaWeight.Full -> FullImage(source, modifier, contentScale, content)
    MediaWeight.Tile -> TileImage(source, modifier, contentScale, content)
  }
}

@Composable
private fun FullImage(source: MediaSource, modifier: Modifier, contentScale: ContentScale, content: @Composable (MediaState) -> Unit) {
  val context = LocalContext.current
  val density = LocalDensity.current.density
  val environment = LocalMediaEnvironment.current
  val media = environment.media
  val screen = rememberScreenVisibility()
  if (screen.visible) declarePlace(source, playing = null, muted = null)
  val tones = rememberCoroutineScope()
  BoxWithConstraints(modifier.then(screen.modifier)) {
    if (!screen.visible) return@BoxWithConstraints
    val pixels = decodePixels(source, constraints, density)
    val scale = contentScale.toScale()
    val request = remember(source, pixels, scale) { imageRequest(context, source, pixels, scale) }
    var state: MediaState by remember(source.url) { mutableStateOf(MediaState.Idle) }
    val blur by produceState<ImageBitmap?>(null, media, source, state == MediaState.Ready) {
      val placeholder = media.placeholder(source.toItem(), state == MediaState.Ready)
      value = (placeholder as? Placeholder.BlurHash)?.let { decodeBlur(it.hash) }
    }
    blur?.let { Image(it, "blurhash", Modifier.fillMaxSize(), contentScale = contentScale) }
    AsyncImage(
      model = request,
      contentDescription = null,
      imageLoader = environment.imageLoader,
      contentScale = contentScale,
      onState = { painterState ->
        val reached = painterState.toMediaState()
        if (reached != null) {
          state = reached
          media.itemState(source.url, reached)
          if (painterState is AsyncImagePainter.State.Success) tones.launch { ToneFrames.submit(media, source.toItem(), painterState.result.drawable) }
        }
      },
      modifier = Modifier.fillMaxSize()
    )
    content(state)
  }
}

@Composable
private fun TileImage(source: MediaSource, modifier: Modifier, contentScale: ContentScale, content: @Composable (MediaState) -> Unit) {
  val context = LocalContext.current
  val environment = LocalMediaEnvironment.current
  val media = environment.media
  val screen = rememberScreenVisibility()
  if (screen.visible) claimTile(source)
  val scale = contentScale.toScale()
  val request = remember(source, scale) { imageRequest(context, source, source.size?.let { (width, height) -> PixelSize(width.toUInt(), height.toUInt()) }, scale) }
  var state: MediaState by remember(source.url) { mutableStateOf(MediaState.Idle) }
  Box(modifier.then(screen.modifier)) {
    if (!screen.visible) return@Box
    if (state != MediaState.Ready) TileBlur(source, contentScale)
    AsyncImage(
      model = request,
      contentDescription = null,
      imageLoader = environment.imageLoader,
      contentScale = contentScale,
      onState = { painterState ->
        painterState.toMediaState()?.let { reached ->
          state = reached
          media.itemState(source.url, reached)
        }
      },
      modifier = Modifier.fillMaxSize()
    )
    content(state)
  }
}

@Composable
private fun TileBlur(source: MediaSource, contentScale: ContentScale) {
  val media = LocalMediaEnvironment.current.media
  val blur by produceState<ImageBitmap?>(null, media, source) {
    value = (media.placeholder(source.toItem(), false) as? Placeholder.BlurHash)?.let { decodeBlur(it.hash) }
  }
  blur?.let { Image(it, "blurhash", Modifier.fillMaxSize(), contentScale = contentScale) }
}

private fun decodePixels(source: MediaSource, constraints: Constraints, density: Float): PixelSize? {
  val declared = source.size?.let { (width, height) -> PixelSize(width.toUInt(), height.toUInt()) }
  if (declared != null) return declared
  if (!constraints.hasBoundedWidth || !constraints.hasBoundedHeight) return null
  return imagePixelSize(constraints.maxWidth / density.toDouble(), constraints.maxHeight / density.toDouble(), density.toDouble(), null)
}

internal fun imageRequest(context: Context, source: MediaSource, pixels: PixelSize?, scale: Scale): ImageRequest =
  ImageRequest.Builder(context)
    .data(MediaImageData(source.url, source.headers))
    .scale(scale)
    .apply {
      pixels?.let {
        size(it.width.toInt(), it.height.toInt())
        memoryCacheKey(MemoryCache.Key("${source.url}|${it.width}x${it.height}|$scale"))
      }
    }
    .build()

internal fun ContentScale.toScale(): Scale = when (this) {
  ContentScale.Fit, ContentScale.Inside, ContentScale.FillWidth, ContentScale.FillHeight -> Scale.FIT
  else -> Scale.FILL
}

private suspend fun decodeBlur(hash: String): ImageBitmap? = withContext(Dispatchers.Default) {
  runCatching {
    val pixels = decodeBlurhash(hash, BLUR_SIZE.toUInt(), BLUR_SIZE.toUInt(), 1f)
    Bitmap.createBitmap(BLUR_SIZE, BLUR_SIZE, Bitmap.Config.ARGB_8888).apply { copyPixelsFromBuffer(ByteBuffer.wrap(pixels)) }.asImageBitmap()
  }.getOrNull()
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
