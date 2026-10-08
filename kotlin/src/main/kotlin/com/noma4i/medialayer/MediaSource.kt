package com.noma4i.medialayer

interface MediaSource {
    val url: String
    val kind: Kind
    val headers: Map<String, String>
}
