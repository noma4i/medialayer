package com.noma4i.medialayer.ui

import com.noma4i.medialayer.MediaState

/** A failure belongs to the full item of the mount that raised it; another item of the same URL does not inherit it. */
internal class ScopedFailure(val key: Any, val state: MediaState)

internal fun ScopedFailure?.of(key: Any): MediaState? = this?.state?.takeIf { this.key == key }
