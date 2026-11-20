package com.noma4i.medialayer.ui

import com.noma4i.medialayer.MediaException
import com.noma4i.medialayer.OutgoingUpload
import com.noma4i.medialayer.RemoteResult
import com.noma4i.medialayer.outgoingFetchSlots
import java.io.ByteArrayInputStream
import java.io.File
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.sync.Semaphore

/** The fetch of the remote copy of an outgoing item: the whole body into the core cache and a decode check. */
fun interface OutgoingRemote {
  /** `Complete` only when the body is whole and decodes; `NotReady` for "the server has no copy yet"; `Failed` is final for the url. [progress] may be reported while fetching. */
  suspend fun fetch(url: String, progress: (Double) -> Unit): RemoteResult
}

/** The time the 5 s re-ask runs on. The testing module gives a manual one. */
interface OutgoingClock {
  suspend fun sleep(millis: Long)
}

internal object SystemOutgoingClock : OutgoingClock {
  override suspend fun sleep(millis: Long) = kotlinx.coroutines.delay(millis)
}

/** The remote through the open layer: the core cache fetches the complete body, then a decode proves it is a picture. */
internal object LayerOutgoingRemote : OutgoingRemote {
  override suspend fun fetch(url: String, progress: (Double) -> Unit): RemoteResult {
    val environment = ProcessMediaEnvironment.current ?: return RemoteResult.NotReady
    return try {
      val fetched = environment.media.fetchImage(url, emptyMap())
      decodeImage(ImageInput { ByteArrayInputStream(fetched.bytes) }, VERIFY_SIDE, environment.media.decodeAdmit()).recycle()
      RemoteResult.Complete
    } catch (error: CancellationException) {
      throw error
    } catch (error: MediaException) {
      val status = statusOf(error.message)
      if (status != null && status in NOT_READY) RemoteResult.NotReady else RemoteResult.Failed(status?.toUShort(), error.message ?: error.toString())
    } catch (error: Exception) {
      RemoteResult.Failed(null, error.message ?: error.toString())
    }
  }

  private val NOT_READY = setOf(404, 202, 425)
  private const val VERIFY_SIDE = 64
  private val STATUS = Regex("status (\\d{3})")
  internal fun statusOf(message: String?): Int? = message?.let { STATUS.find(it)?.groupValues?.get(1)?.toInt() }
}

/** The process state of outgoing media: the registry, the fetch slots, the remote and clock, the custody store. */
internal class OutgoingRuntime(
  internal val remote: OutgoingRemote,
  internal val clock: OutgoingClock,
  private val storeDirectory: () -> File?
) {
  internal val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
  internal val slots = Semaphore(outgoingFetchSlots().toInt())
  private val items = LinkedHashMap<String, OutgoingEntry>(16, 0.75f, true)

  internal fun store(): OutgoingStore? = storeDirectory()?.let { OutgoingStore(it) }

  internal fun item(key: String, create: () -> OutgoingEntry): OutgoingEntry = synchronized(items) {
    val found = items[key]
    if (found != null) { found.touch(); return found }
    create().also { items[key] = it; trim() }
  }

  internal fun find(key: String): OutgoingEntry? = synchronized(items) { items[key] }

  private fun trim() {
    if (items.size <= KEPT) return
    val iterator = items.entries.iterator()
    while (items.size > KEPT && iterator.hasNext()) if (iterator.next().value.phase == com.noma4i.medialayer.OutgoingPhase.Ready) iterator.remove()
  }
}

/** Ready items are tiny, but not kept for ever: the oldest ready ones go past this many. */
private const val KEPT = 512

internal object Outgoing {
  @Volatile internal var runtime: OutgoingRuntime = OutgoingRuntime(LayerOutgoingRemote, SystemOutgoingClock, ::layerStoreDirectory)

  /** For the testing module and apps with their own fetch: replaces the remote, the clock and the custody directory, and starts an empty registry. */
  fun install(remote: OutgoingRemote, clock: OutgoingClock = SystemOutgoingClock, storeDirectory: File? = null) {
    runtime = OutgoingRuntime(remote, clock) { storeDirectory }
  }

  fun reset() {
    runtime = OutgoingRuntime(LayerOutgoingRemote, SystemOutgoingClock, ::layerStoreDirectory)
  }
}

/** Opt-in entry of the testing module: replaces the remote, the clock and the custody directory, and starts an empty registry. */
@MediaLayerTestingApi
fun installOutgoing(remote: OutgoingRemote, clock: OutgoingClock, storeDirectory: File) = Outgoing.install(remote, clock, storeDirectory)

/** Opt-in entry of the testing module: forgets every item and restores the system remote, clock and store. */
@MediaLayerTestingApi
fun resetOutgoing() = Outgoing.reset()

private fun layerStoreDirectory(): File? {
  val context = ProcessMediaEnvironment.current?.media?.binding?.factory?.appContext ?: return null
  return File(context.cacheDir, "medialayer/outgoing")
}

/** The item registry by key: the item (and its phase) is readable outside any composition, until a relaunch. */
fun Media.outgoing(key: String): OutgoingMedia? = Outgoing.runtime.find(key)?.let { OutgoingMedia(it) }
