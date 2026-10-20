package com.noma4i.medialayer.ui

import android.content.Context
import android.graphics.Bitmap
import android.graphics.drawable.Animatable
import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.State
import androidx.compose.runtime.getValue
import androidx.compose.runtime.produceState
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Modifier
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.Constraints
import coil.compose.AsyncImage
import coil.compose.AsyncImagePainter
import coil.ImageLoader
import coil.request.ImageResult
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
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.awaitCancellation
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.transformLatest
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

@Composable
fun MediaImage(
  source: MediaSource,
  modifier: Modifier = Modifier,
  contentScale: ContentScale = ContentScale.Crop,
  content: @Composable (MediaState) -> Unit = {}
) {
  val screen = rememberScreenVisibility()
  claimImage(source, screen)
  val size = source.size
  if (size != null) {
    Box(modifier.then(screen.modifier)) {
      ImageBody(source, PixelSize(size.first.toUInt(), size.second.toUInt()), screen, contentScale, content)
    }
  } else {
    val density = LocalDensity.current.density
    BoxWithConstraints(modifier.then(screen.modifier)) {
      ImageBody(source, decodePixels(constraints, density), screen, contentScale, content)
    }
  }
}

@Composable
private fun claimImage(source: MediaSource, screen: ScreenVisibility) {
  val media = LocalMediaEnvironment.current.media
  LaunchedEffect(media, source, screen) {
    val owner = Any()
    val item = source.toItem()
    snapshotFlow { screen.visible }.collectLatest { visible ->
      if (visible) {
        media.claims.visible(owner, item, true)
        try { awaitCancellation() } finally { media.claims.release(owner, item.url) }
      }
    }
  }
}

@Composable
@OptIn(ExperimentalCoroutinesApi::class)
private fun ImageBody(source: MediaSource, pixels: PixelSize?, screen: ScreenVisibility, contentScale: ContentScale, content: @Composable (MediaState) -> Unit) {
  val context = LocalContext.current
  val environment = LocalMediaEnvironment.current
  val media = environment.media
  val tones = rememberCoroutineScope()
  val scale = contentScale.toScale()
  val request = remember(media, source, pixels, scale) { imageRequest(context, source, pixels, scale, media.cacheKey(source.url, source.headers)) }
  val state = remember(source.url) { mutableStateOf<MediaState>(MediaState.Idle) }
  val animation = remember { mutableStateOf<Animatable?>(null) }
  LaunchedEffect(screen) {
    snapshotFlow { animation.value to screen.visible }.collectLatest { (drawable, visible) ->
      if (visible) drawable?.start() else drawable?.stop()
      try { awaitCancellation() } finally { drawable?.stop() }
    }
  }
  val loader = remember(environment.imageLoader, screen) {
    object : ImageLoader by environment.imageLoader {
      override suspend fun execute(request: ImageRequest): ImageResult =
        // Visibility cancels in-flight decoding without invalidating the image composition.
        snapshotFlow { screen.visible }.transformLatest { visible ->
          if (visible) emit(environment.imageLoader.execute(request))
        }.first()
    }
  }
  ImageBlur(source, screen, state, contentScale)
  AsyncImage(
    model = request,
    contentDescription = null,
    imageLoader = loader,
    contentScale = contentScale,
    onState = { painterState ->
      animation.value = (painterState as? AsyncImagePainter.State.Success)?.result?.drawable as? Animatable
      painterState.toMediaState()?.let { reached ->
        state.value = reached
        media.itemState(source.url, reached)
        if (painterState is AsyncImagePainter.State.Success) tones.launch { ToneFrames.submit(media, source.toItem(), painterState.result.drawable) }
      }
    },
    modifier = Modifier.fillMaxSize()
  )
  ImageContent(screen, state, content)
}

@Composable
private fun ImageContent(screen: ScreenVisibility, state: State<MediaState>, content: @Composable (MediaState) -> Unit) {
  if (screen.visible) content(state.value)
}

@Composable
private fun ImageBlur(source: MediaSource, screen: ScreenVisibility, state: State<MediaState>, contentScale: ContentScale) {
  if (!screen.visible || state.value == MediaState.Ready) return
  val media = LocalMediaEnvironment.current.media
  val blur by produceState<ImageBitmap?>(null, media, source) {
    value = (media.placeholder(source.toItem(), false) as? Placeholder.BlurHash)?.let { decodeBlur(it.hash) }
  }
  blur?.let { Image(it, "blurhash", Modifier.fillMaxSize(), contentScale = contentScale) }
}

private fun decodePixels(constraints: Constraints, density: Float): PixelSize? {
  if (!constraints.hasBoundedWidth || !constraints.hasBoundedHeight) return null
  return imagePixelSize(constraints.maxWidth / density.toDouble(), constraints.maxHeight / density.toDouble(), density.toDouble(), null)
}

internal fun imageRequest(context: Context, source: MediaSource, pixels: PixelSize?, scale: Scale, cacheKey: String): ImageRequest =
  ImageRequest.Builder(context)
    .data(MediaImageData(source.url, source.headers, cacheKey))
    .scale(scale)
    .apply {
      pixels?.let {
        size(it.width.toInt(), it.height.toInt())
        memoryCacheKey(MemoryCache.Key("$cacheKey|${it.width}x${it.height}|$scale"))
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
