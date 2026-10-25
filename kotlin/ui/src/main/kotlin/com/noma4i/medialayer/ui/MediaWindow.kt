package com.noma4i.medialayer.ui

import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.foundation.lazy.grid.LazyGridState
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.snapshotFlow
import com.noma4i.medialayer.MediaException
import com.noma4i.medialayer.MediaItem
import com.noma4i.medialayer.MediaLayer
import com.noma4i.medialayer.MediaWindow as CoreWindow
import com.noma4i.medialayer.WindowReadyListener
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

internal sealed interface WindowLayout<Row> {
  class Video<Row>(val source: (Row) -> MediaItem?) : WindowLayout<Row>
  class Image<Row>(val name: String, val rowsPerPage: Int, val reversed: Boolean, val sources: (Row) -> List<MediaItem>) : WindowLayout<Row>
}

internal data class WindowUpdate<Row>(
  val rows: List<Row>? = null, val active: Int? = null, val visible: IntRange? = null, val isActive: Boolean? = null, val closes: Boolean = false
) {
  companion object {
    fun <Row> merged(held: WindowUpdate<Row>?, next: WindowUpdate<Row>) = WindowUpdate(
      next.rows ?: held?.rows, next.active ?: held?.active, next.visible ?: held?.visible, next.isActive ?: held?.isActive, (held?.closes ?: false) || next.closes
    )
  }
}

/** Follows one window of the app list: one running preparation and one latest snapshot, mapped and sent to the core off the main thread. */
internal class WindowFollower<Row>(
  private val media: MediaLayer, private val layout: WindowLayout<Row>, private val onReady: ((Boolean) -> Unit)? = null, failed: (MediaException) -> Unit
) {
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

  fun show(rows: IntRange) = drain.submit(WindowUpdate(visible = rows))

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
        is WindowLayout.Image -> media.openImageWindow(layout.name, layout.rowsPerPage.toUInt(), layout.reversed)
      }
      onReady?.let { report -> window?.subscribeReady(object : WindowReadyListener { override fun changed(ready: Boolean) = report(ready) }) }
    }
    val opened = window ?: return
    update.active?.let { active = it }
    val index = active.coerceAtLeast(0).toUInt()
    val rows = update.rows
    when (layout) {
      is WindowLayout.Video -> when {
        rows != null -> opened.replace(index, rows.map(layout.source))
        update.active != null -> opened.moveTo(index)
      }
      is WindowLayout.Image -> {
        rows?.let { opened.replaceRows(it.map(layout.sources)) }
        val visible = update.visible
        when {
          visible != null -> opened.showRows(visible.first.coerceAtLeast(0).toUInt(), visible.last.coerceAtLeast(0).toUInt())
          update.active != null -> opened.moveTo(index)
        }
      }
    }
    update.isActive?.let(opened::setActive)
  }
}

private fun visibleRows(indices: List<Int>): IntRange? = if (indices.isEmpty()) null else indices.min()..indices.max()

private sealed interface WindowPlacement {
  class Page(val index: Int) : WindowPlacement
  class Rows(val visible: () -> IntRange?) : WindowPlacement
}

@Composable
private fun <Row> FollowWindow(layout: WindowLayout<Row>, data: () -> List<Row>, placement: WindowPlacement, enabled: Boolean, onFailed: (MediaException) -> Unit): Boolean {
  if (currentEnvironmentOrNull() == null) return false
  val media = LocalMediaEnvironment.current.media
  val failed by rememberUpdatedState(onFailed)
  val currentData by rememberUpdatedState(data)
  val ready = remember(media, layout) { mutableStateOf(false) }
  val follower = remember(media, layout) { WindowFollower(media, layout, { ready.value = it }) { failed(it) } }
  DisposableEffect(follower) { onDispose { follower.close() } }
  LaunchedEffect(follower, enabled) { follower.setActive(enabled) }
  when (placement) {
    is WindowPlacement.Page -> LaunchedEffect(follower, placement.index) { follower.move(placement.index) }
    is WindowPlacement.Rows -> {
      val visible by rememberUpdatedState(placement.visible)
      LaunchedEffect(follower) { snapshotFlow { visible() }.collect { rows -> if (rows != null) follower.show(rows) } }
    }
  }
  LaunchedEffect(follower) {
    withContext(Dispatchers.Default) { snapshotFlow { currentData() }.collect { follower.replace(it) } }
  }
  return ready.value
}

/** The latest mapper serves the next send; the one it replaced is released. */
internal class LatestMapper<In, Out>(initial: (In) -> Out) {
  @Volatile var current: (In) -> Out = initial

  operator fun invoke(row: In): Out = current(row)
}

/**
 * Declares the video window of a feed. `data` reads snapshot state; `source` is a pure function of a row; both run off the main thread.
 * The latest `source` serves the next send of rows; rows already sent are not mapped again.
 * Returns true while the first frame of the active source has been rendered or the source is cached, and false again after every change of `active` or of the active source.
 */
@Composable
fun <Row> MediaWindow(data: () -> List<Row>, source: (Row) -> MediaItem?, active: Int, enabled: Boolean = true, onFailed: (MediaException) -> Unit = {}): Boolean {
  val mapper = remember { LatestMapper(source) }
  SideEffect { mapper.current = source }
  return FollowWindow(remember { WindowLayout.Video(mapper::invoke) }, data, WindowPlacement.Page(active), enabled, onFailed)
}

/**
 * Declares a named image window. `sources` is a pure function of a row. `rowsPerPage` neighbouring rows form one page and `active` is a page index;
 * a `reversed` list counts its pages from the last row. Returns true while every source of the active page is cached or failed, and false again after every change of the active page.
 */
@Composable
fun <Row> MediaImageWindow(
  name: String, data: () -> List<Row>, sources: (Row) -> List<MediaItem>, active: Int, rowsPerPage: Int = 1, reversed: Boolean = false, enabled: Boolean = true,
  onFailed: (MediaException) -> Unit = {}
): Boolean = imageWindow(name, data, sources, rowsPerPage, reversed, WindowPlacement.Page(active), enabled, onFailed)

/**
 * Declares a named image window over a lazy list whose item `index` is the row `index`. The active page is the page of the first visible row, the last one in a `reversed` list;
 * the window does not move while no row is visible. Returns the readiness of the active page.
 */
@Composable
fun <Row> MediaImageWindow(
  name: String, data: () -> List<Row>, sources: (Row) -> List<MediaItem>, state: LazyListState, rowsPerPage: Int = 1, reversed: Boolean = false,
  enabled: Boolean = true, onFailed: (MediaException) -> Unit = {}
): Boolean = imageWindow(
  name, data, sources, rowsPerPage, reversed,
  WindowPlacement.Rows { visibleRows(state.layoutInfo.visibleItemsInfo.map { it.index }) }, enabled, onFailed
)

/** The same over a lazy grid whose item `index` is the row `index`. */
@Composable
fun <Row> MediaImageWindow(
  name: String, data: () -> List<Row>, sources: (Row) -> List<MediaItem>, state: LazyGridState, rowsPerPage: Int = 1, reversed: Boolean = false,
  enabled: Boolean = true, onFailed: (MediaException) -> Unit = {}
): Boolean = imageWindow(
  name, data, sources, rowsPerPage, reversed,
  WindowPlacement.Rows { visibleRows(state.layoutInfo.visibleItemsInfo.map { it.index }) }, enabled, onFailed
)

@Composable
private fun <Row> imageWindow(
  name: String, data: () -> List<Row>, sources: (Row) -> List<MediaItem>, rowsPerPage: Int, reversed: Boolean, placement: WindowPlacement, enabled: Boolean,
  onFailed: (MediaException) -> Unit
): Boolean {
  require(rowsPerPage > 0) { "rowsPerPage must be positive" }
  val mapper = remember { LatestMapper(sources) }
  SideEffect { mapper.current = sources }
  return FollowWindow(remember(name, rowsPerPage, reversed) { WindowLayout.Image(name, rowsPerPage, reversed, mapper::invoke) }, data, placement, enabled, onFailed)
}
