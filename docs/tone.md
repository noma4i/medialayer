# Tone

MediaLayer считает цвет только после запроса `tone(source, band)` или подписки `subscribeTone`. Ядро хранит результат вместе с файлом кэша. Приложение не запускает подготовку пикселей.

| Question | Answer |
| -------- | ------ |
| Кто готовит пиксели | Android использует небольшой программный декод Coil, iOS использует ImageIO thumbnail. Для видео платформа извлекает уменьшенный кадр |
| Когда готов тон | BlurHash доступен сразу. После появления спроса ядро вызывает библиотечный callback с источником и уведомляет подписчиков |
| What does a tone hold | An 8 by 12 grid of colors, the average color of the whole frame, the average of a top band, the average of the rest, up to 3 dominant colors |
| Where is it kept | With the cache entry of the source. It survives a restart and goes away with the entry |
| How big is it | 311 bytes per entry |

## Read a tone

`tone(source, band)` answers with the tone of the source, or nil when nothing is known yet. `band` is the share of the height that counts as the top, from just above 0 to 1. A top bar of 15 percent of the frame uses `band` 0.15. The same stored tone answers every `band`.

The answer comes from the best source in this order: pixels of the cache entry, then the BlurHash of the source. A BlurHash tone is marked `blurHash` in `source`, a pixel tone `pixels`. A source without both answers nil, never an invented color.

### Swift

```swift
if let tone = try await media.tone(source: item, band: 0.15) {
    let top = tone.top.color          // Rgb with r, g, b
    let isLight = tone.top.luminance > 0.5
    let accents = tone.dominant       // up to 3, ordered by share
}
```

### Kotlin

```kotlin
val tone = media.tone(item, 0.15f)
if (tone != null) {
    val top = tone.top.color          // Rgb with r, g, b
    val isLight = tone.top.luminance > 0.5
    val accents = tone.dominant       // up to 3, ordered by share
}
```

## React to a new tone

Тон пикселей заменяет BlurHash после появления спроса. Поздняя подписка запускает подготовку источника без повторного появления вида. `subscribeTone(source, listener)` учитывает заголовки источника и сообщает об изменении тона. Несколько изменений могут объединиться в одно уведомление. Храните подписку, пока нужны обновления.

### Swift

```swift
final class Bar: ToneListener {
    func changed(source: ToneSource) { /* ask tone(source:band:) again */ }
}
let subscription = media.subscribeTone(source: item, listener: bar)
```

### Kotlin

```kotlin
val subscription = media.subscribeTone(item, object : ToneListener {
    override fun changed(source: ToneSource) { /* ask tone(source, band) again */ }
})
```

## What you do not do

- Приложение не декодирует изображения для тона. Перед подготовкой платформа спрашивает `tonePixelsNeeded`, затем уменьшает изображение до 64 пикселей по большей стороне.
- Без спроса и при готовом пиксельном тоне подготовок нет. Ядро допускает только одну подготовку на ключ одновременно.
- Ошибка освобождает запрос. Уход последнего подписчика отменяет подготовку, если нет незавершённого запроса `tone`. Повторный спрос запускает её снова.
- You do not store the tone yourself. To keep it with a row of your database, write it into a local field of that row (see the local fields section of the DBlayer models document).
- You do not run the analysis twice for one source: the first pixels of a source win, later ones are dropped.

## Изменения API

`submitTonePixels` удалён. Библиотека устанавливает один `TonePreparation` через `setTonePreparation` при открытии UI. Метод `prepare(source)` возвращает `TonePixels` и не хранит пиксели между вызовами.

Swift: `subscribeTone(url:listener:)` заменён на `subscribeTone(source:listener:)`. Kotlin: `subscribeTone(url, listener)` заменён на `subscribeTone(source, listener)`. Передавайте `MediaItem` с заголовками вместо URL.

Сигнатуры `tone(source:band:)` и `tone(source, band)` сохранены. При отсутствии пиксельного результата запрос запускает подготовку. Тестовые реализации `MediaPlaces` реализуют `setTonePreparation`, `tonePixelsNeeded` и `playUrl`.
