# MediaLayerProbe

A micro app that checks the library on a real iPhone with the real camera and microphone. The tests use only the public API the app uses
(`MediaCamera`, `MediaCameraPreview`, `MediaVoiceRecorder`); the only seam is `MediaTrace.observer`, which reads the trace lines of the library.

Run: copy `Local.xcconfig.example` to `Local.xcconfig` (untracked) and give your team, unlock the phone, then `bash scripts/check_device_ios.sh <device UDID>`.
Keep the phone unlocked and the app in front: the camera runs only then. On the first run tap Allow for the camera and for the microphone.

Checks: `open` (READY in 5 s), `take_front_5s` (circle), `take_back_5s` (moment), `take_front_15s` (pause after stop not above 1.5 s), `trace_rules` (angle, no frameOutput commit while bound, audio track, no rewrite),
`coverage` (the coverage line as printed, `coverage=absent` before it exists; no take refused), `voice_5s`.
The run is saved under `target/device-ios/<time>/`, with `files/` holding the clips, `probe.log` and the first frame of every clip as PNG:
look at the PNGs by eye, the orientation of a face is not a number.
