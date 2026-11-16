# Release profile

The root Cargo.toml sets codegen-units = 1.
The default opt-level = 3, LTO = false and panic = "unwind" stay.
UniFFI catches panics, so panic = "abort" is not allowed.

The comparison covers Android arm64-v8a, armeabi-v7a and iOS device.
On the same sources codegen-units = 1 makes all 3 files smaller.
LTO barely changes the Android .so and makes the iOS .a larger.
opt-level = "s" makes all 3 files larger than opt-level = 3.
The counters of processed bytes and segments are equal.
Timings under background load do not prove a speedup.

strip = "symbols" removes the UniFFI metadata from the iOS .a.
strip = "debuginfo" did not change the checked files compared to the default profile without debug info.
Extra symbol stripping is off, so names stay available for diagnostics.
After a profile change, check bindgen and that medialayer.swift and medialayer.kt are equal.
Check the size of the final app apart from the size of the static library.
