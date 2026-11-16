# Release build checks

A MediaLayer object without an owner is released through Cleaner.
A test holds the object until the checked operation ends and calls close in finally.
This keeps the owner alive when the optimizer changes local variables.

UniFFI registers native methods through Native.register.
R8 keeps their names, the JNA structures and the callback methods.
Plain models do not need to keep the whole package.

MediaSource allows mutable implementations and a Map of headers.
UniFFI creates MediaItem with mutable fields.
Do not declare these types immutable through a stability config.
MediaImage, MediaVideo and MediaAudio allow composition skipping when strong skipping is on.
