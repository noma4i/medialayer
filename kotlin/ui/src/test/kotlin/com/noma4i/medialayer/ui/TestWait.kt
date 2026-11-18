package com.noma4i.medialayer.ui

/**
 * The bound of a Compose test wait. The conditions waited on are events (a Coil fetch, a layer call, a semantics node); the bound only
 * ends a hung test. A run on a loaded machine pays the first Coil and Robolectric start inside the first wait, so it is far above that cost.
 */
internal const val COMPOSE_WAIT_MS = 60_000L
