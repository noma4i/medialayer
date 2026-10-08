# Memory control

MediaLayer owns the memory of your players. It reuses a fixed set of player slots, frees buffers and decoders as soon as a slot is released, and shrinks everything when the system is under pressure. It is built for low-end Android devices.

| Feature | Meaning |
| ------- | ------- |
| Memory budget | One configurable limit for players, proxy buffers and image memory |
| Slot recycling | Players are reused, never created per screen; an unloaded slot holds no frames or buffers |
| Retention levels | `hot` plays or is next, `warm` keeps metadata and a small buffer, `cold` holds nothing |
| Memory pressure | Report `moderate` or `critical`; prefetch stops first, far slots are released next, the playing slot last |
| Device profile | Low-memory devices get fewer slots, shorter buffers, lower bitrate and fewer parallel loads |
| Background | Everything except the chosen slot is released and prefetch stops |
| Statistics | Slots per level, estimated memory and release counts in `stats()` |

Report pressure from the system callbacks.

### Swift

```swift
NotificationCenter.default.addObserver(forName: UIApplication.didReceiveMemoryWarningNotification,
                                       object: nil, queue: nil) { _ in
  media.memoryPressure(.critical)
}
```

### Kotlin

```kotlin
override fun onTrimMemory(level: Int) {
  super.onTrimMemory(level)
  media.memoryPressure(if (level >= TRIM_MEMORY_RUNNING_CRITICAL) Pressure.Critical else Pressure.Moderate)
}
```

The device profile is read once at open: `ram` and `isLowRam` on Android, physical memory on iOS.
