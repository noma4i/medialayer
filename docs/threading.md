# Threading and performance

MediaLayer never blocks your threads. Every public call either returns at once or is `async` (Swift) / `suspend` (Kotlin). Network, disk, parsing and eviction run on library threads, in parallel, and low-priority work is deferred until the app is idle.

| Guarantee | Meaning |
| --------- | ------- |
| Calls are cheap | Calls like `prefetch` and `cancel` only queue work |
| Many requests at once | The proxy serves many players in parallel; one slow response never delays another |
| Disk never blocks | Cache reads and writes run off the runtime threads |
| Active playback first | Loading what is playing always goes before prefetch |
| Deferred work | Eviction, checks and far prefetch wait for idle time |
| Instant open | Opening does not scan the disk |
| Listeners | Called from library threads with the latest state only; hop to the main thread yourself |
| Cancel | Stops work at once and releases connections and files |
