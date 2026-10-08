package com.noma4i.medialayer.ui

import android.content.Context
import androidx.media3.common.MediaItem
import androidx.media3.common.util.Util
import androidx.media3.datasource.DataSource
import androidx.media3.datasource.DefaultDataSource
import androidx.media3.datasource.okhttp.OkHttpDataSource
import androidx.media3.exoplayer.hls.HlsMediaSource
import androidx.media3.exoplayer.source.MediaSource
import androidx.media3.exoplayer.source.ProgressiveMediaSource
import com.noma4i.medialayer.PlaybackRoute
import okhttp3.Call

internal class HttpMediaSources(private val context: Context, private val callFactory: Call.Factory) {
  fun dataSourceFactory(headers: Map<String, String>): DataSource.Factory {
    val http = OkHttpDataSource.Factory(callFactory)
      .setUserAgent(Util.getUserAgent(context, "MediaLayer"))
      .setDefaultRequestProperties(headers)
    return DefaultDataSource.Factory(context, http)
  }

  fun create(route: PlaybackRoute): MediaSource {
    val item = MediaItem.fromUri(route.url)
    val dataSources = dataSourceFactory(route.headers)
    if (route.hls) return HlsMediaSource.Factory(dataSources).createMediaSource(item)
    return ProgressiveMediaSource.Factory(dataSources).createMediaSource(item)
  }
}
