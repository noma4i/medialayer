package com.noma4i.medialayer.ui

import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.staticCompositionLocalOf
import com.noma4i.medialayer.MediaSource
import com.noma4i.medialayer.normalizeItem
import com.noma4i.medialayer.toItem
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

internal val LocalSourceKey = staticCompositionLocalOf<(MediaSource) -> String> { { normalizeItem(it.toItem()).url } }

/** The key of the core for a source, read once per `source.url` off the main thread. `null` until it is read, so a lookup by the declared text never happens. */
@Composable
internal fun rememberSourceKey(source: MediaSource): String? {
  val read = LocalSourceKey.current
  var key by remember(source.url, read) { mutableStateOf<String?>(null) }
  LaunchedEffect(source.url, read) { key = withContext(Dispatchers.Default) { read(source) } }
  return key
}
