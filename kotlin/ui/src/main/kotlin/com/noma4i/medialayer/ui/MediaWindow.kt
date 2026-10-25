package com.noma4i.medialayer.ui

import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.snapshotFlow
import com.noma4i.medialayer.MediaException
import com.noma4i.medialayer.MediaItem
import com.noma4i.medialayer.MediaLayer
import com.noma4i.medialayer.MediaWindow as CoreWindow
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

internal sealed interface WindowLayout<Row> {
  class Video<Row>(val source: (Row) -> MediaItem?) : WindowLayout<Row>
  class Image<Row>(val name: String, val rowsPerPage: Int, val sources: (Row) -> List<MediaItem>) : WindowLayout<Row>
}

internal data class WindowUpdate<Row>(val rows: List<Row>? = null, val active: Int? = null, val isActive: Boolean? = null, val closes: Boolean = false) {
  companion object {
    fun <Row> merged(held: WindowUpdate<Row>?, next: WindowUpdate<Row>) =
      WindowUpdate(next.rows ?: held?.rows, next.active ?: held?.active, next.isActive ?: held?.isActive, (held?.closes ?: false) || next.closes)
  }
}

/** Follows one window of the app list: one running preparation and one latest snapshot, mapped and sent to the core off the main thread. */
internal class WindowFollower<Row>(private val media: MediaLayer, private val layout: WindowLayout<Row>, failed: (MediaException) -> Unit) {
  private var window: CoreWindow? = null
  private var active = 0
  private var closed = false
  private val drain = LatestDrain<WindowUpdate<Row>>({ held, next -> WindowUpdate.merged(held, next) }) { update ->
    try {
      applyUpdate(update)
    } catch (error: MediaException) {
      failed(error)
    }
  }

  fun replace(rows: List<Row>) = drain.submit(WindowUpdate(rows = rows))

  fun move(to: Int) = drain.submit(WindowUpdate(active = to))

  fun setActive(isActive: Boolean) = drain.submit(WindowUpdate(isActive = isActive))

  fun close() = drain.submit(WindowUpdate(closes = true))

  private fun applyUpdate(update: WindowUpdate<Row>) {
    if (closed) return
    if (update.closes) {
      closed = true
      window?.let { opened ->
        try { opened.release() } finally { opened.close() }
      }
      window = null
      return
    }
    if (window == null) {
      window = when (layout) {
        is WindowLayout.Video -> media.openVideoWindow()
        is WindowLayout.Image -> media.openImageWindow(layout.name)
      }
    }
    val opened = window ?: return
    update.active?.let { active = it }
    val index = active.coerceAtLeast(0).toUInt()
    val rows = update.rows
    when {
      rows != null -> when (layout) {
        is WindowLayout.Video -> opened.replace(index, rows.map(layout.source))
        is WindowLayout.Image -> opened.replacePages(index, rows.chunked(layout.rowsPerPage).map { page -> page.flatMap(layout.sources) })
      }
      update.active != null -> opened.moveTo(index)
    }
    update.isActive?.let(opened::setActive)
  }
}

@Composable
private fun <Row> FollowWindow(layout: WindowLayout<Row>, data: () -> List<Row>, active: Int, enabled: Boolean, onFailed: (MediaException) -> Unit) {
  val media = LocalMediaEnvironment.current.media
  val failed by rememberUpdatedState(onFailed)
  val currentData by rememberUpdatedState(data)
  val follower = remember(media, layout) { WindowFollower(media, layout) { failed(it) } }
  DisposableEffect(follower) { onDispose { follower.close() } }
  LaunchedEffect(follower, enabled) { follower.setActive(enabled) }
  LaunchedEffect(follower, active) { follower.move(active) }
  LaunchedEffect(follower) {
    withContext(Dispatchers.Default) { snapshotFlow { currentData() }.collect { follower.replace(it) } }
  }
}

/** The latest mapper serves the next send; the one it replaced is released. */
internal class LatestMapper<In, Out>(initial: (In) -> Out) {
  @Volatile var current: (In) -> Out = initial

  operator fun invoke(row: In): Out = current(row)
}

/**
 * Declares the video window of a feed. `data` reads snapshot state; `source` is a pure function of a row; both run off the main thread.
 * The latest `source` serves the next send of rows; rows already sent are not mapped again.
 */
@Composable
fun <Row> MediaWindow(data: () -> List<Row>, source: (Row) -> MediaItem?, active: Int, enabled: Boolean = true, onFailed: (MediaException) -> Unit = {}) {
  val mapper = remember { LatestMapper(source) }
  SideEffect { mapper.current = source }
  FollowWindow(remember { WindowLayout.Video(mapper::invoke) }, data, active, enabled, onFailed)
}

/** Declares a named image window. `sources` is a pure function of a row. `rowsPerPage` neighbouring rows form one page and `active` is a page index. */
@Composable
fun <Row> MediaImageWindow(
  name: String, data: () -> List<Row>, sources: (Row) -> List<MediaItem>, active: Int, rowsPerPage: Int = 1, enabled: Boolean = true,
  onFailed: (MediaException) -> Unit = {}
) {
  require(rowsPerPage > 0) { "rowsPerPage must be positive" }
  val mapper = remember { LatestMapper(sources) }
  SideEffect { mapper.current = sources }
  FollowWindow(remember(name, rowsPerPage) { WindowLayout.Image(name, rowsPerPage, mapper::invoke) }, data, active, enabled, onFailed)
}
