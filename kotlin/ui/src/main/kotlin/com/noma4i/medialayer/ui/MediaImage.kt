package com.noma4i.medialayer.ui

import android.graphics.Bitmap
import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.produceState
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import coil.compose.AsyncImage
import coil.compose.AsyncImagePainter
import coil.request.ImageRequest
import com.noma4i.medialayer.MediaSource
import com.noma4i.medialayer.MediaState
import com.noma4i.medialayer.Placeholder
import com.noma4i.medialayer.decodeBlurhash
import com.noma4i.medialayer.toItem
import java.nio.ByteBuffer
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

@Composable
fun MediaImage(
  source: MediaSource,
  modifier: Modifier = Modifier,
  contentScale: ContentScale = ContentScale.Crop,
  content: @Composable (MediaState) -> Unit = {}
) {
  val context = LocalContext.current
  val media = LocalMediaEnvironment.current.media
  rememberPlace(source, playing = null, muted = null)
  var state: MediaState by remember(source.url) { mutableStateOf(MediaState.Idle) }
  val request = remember(source) {
    ImageRequest.Builder(context).data(MediaImageData(source.url, source.headers)).apply { source.size?.let { (width, height) -> size(width, height) } }.build()
  }
  val blur by produceState<ImageBitmap?>(null, media, source, state == MediaState.Ready) {
    val placeholder = media.placeholder(source.toItem(), state == MediaState.Ready)
    value = (placeholder as? Placeholder.BlurHash)?.let { decodeBlur(it.hash) }
  }
  Box(modifier) {
    blur?.let { Image(it, "blurhash", Modifier.fillMaxSize(), contentScale = contentScale) }
    AsyncImage(
      model = request,
      contentDescription = null,
      imageLoader = LocalMediaEnvironment.current.imageLoader,
      contentScale = contentScale,
      onState = { painterState ->
        state = painterState.toMediaState()
        media.itemState(source.url, state)
      },
      modifier = Modifier.fillMaxSize()
    )
    content(state)
  }
}

private suspend fun decodeBlur(hash: String): ImageBitmap? = withContext(Dispatchers.Default) {
  runCatching {
    val pixels = decodeBlurhash(hash, BLUR_SIZE.toUInt(), BLUR_SIZE.toUInt(), 1f)
    Bitmap.createBitmap(BLUR_SIZE, BLUR_SIZE, Bitmap.Config.ARGB_8888).apply { copyPixelsFromBuffer(ByteBuffer.wrap(pixels)) }.asImageBitmap()
  }.getOrNull()
}

private const val BLUR_SIZE = 32

private fun AsyncImagePainter.State.toMediaState(): MediaState = when (this) {
  is AsyncImagePainter.State.Empty -> MediaState.Idle
  is AsyncImagePainter.State.Loading -> MediaState.Loading
  is AsyncImagePainter.State.Success -> MediaState.Ready
  is AsyncImagePainter.State.Error -> MediaState.Failed(result.throwable.message ?: result.throwable.javaClass.simpleName)
}
