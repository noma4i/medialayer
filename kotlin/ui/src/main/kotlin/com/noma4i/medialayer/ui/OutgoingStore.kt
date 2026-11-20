package com.noma4i.medialayer.ui

import com.noma4i.medialayer.outgoingStale
import java.io.File
import java.nio.file.Files
import java.security.MessageDigest

/**
 * The custody of the local originals of outgoing items under the MediaLayer cache root, keyed by a hash of the item key.
 * The original is hard-linked (copied when the link fails), so it survives a recycled row and a relaunch, and costs no bytes with a link.
 * An item releases its original at `ready`, one pass at layer open deletes what the core calls stale.
 */
internal class OutgoingStore(private val root: File, private val now: () -> Long = System::currentTimeMillis) {
  private fun directory(key: String) = File(root, MessageDigest.getInstance("SHA-256").digest(key.toByteArray()).joinToString("") { "%02x".format(it) })

  /** The kept original of [key]: the one already held, or a link/copy of [source]. Declaring the same key again keeps the same file. */
  fun adopt(key: String, source: File): File {
    val directory = directory(key).apply { mkdirs() }
    val kept = File(directory, ORIGINAL)
    val meta = File(directory, META)
    val created = if (kept.exists() && meta.exists()) createdOf(meta) else {
      kept.delete()
      try { Files.createLink(kept.toPath(), source.toPath()) } catch (error: Exception) { source.copyTo(kept, overwrite = true) }
      now()
    }
    meta.writeText("$created\n${now()}")
    return kept
  }

  fun release(key: String) {
    directory(key).deleteRecursively()
  }

  fun held(key: String): Boolean = File(directory(key), ORIGINAL).exists()

  /** The one garbage pass: entries the core calls stale (never ready for 7 days, or not declared again for 24 hours) are deleted. */
  fun sweep() {
    val entries = root.listFiles() ?: return
    for (entry in entries) {
      val meta = File(entry, META)
      val times = runCatching { meta.readText().lines().map { it.trim().toLong() } }.getOrNull()
      if (times == null || times.size < 2 || outgoingStale(times[0].toULong(), times[1].toULong(), now().toULong(), false)) entry.deleteRecursively()
    }
  }

  private fun createdOf(meta: File): Long = runCatching { meta.readText().lines().first().trim().toLong() }.getOrDefault(now())

  companion object {
    private const val ORIGINAL = "original"
    private const val META = "meta"
    fun sweep(root: File) = OutgoingStore(root).sweep()
  }
}
