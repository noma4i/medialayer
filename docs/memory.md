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
