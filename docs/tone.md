# Tone

MediaLayer считает цвет только после запроса `tone(source, band)` или подписки `WithMediaTone`, `rememberMediaTone`, `subscribeTone`. Ядро хранит результат вместе с файлом кэша. Приложение не запускает подготовку пикселей.

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

Тон пикселей заменяет BlurHash после появления спроса. Поздняя подписка запускает подготовку источника без повторного появления вида. В интерфейсе подписку держит `WithMediaTone` (Swift) или `rememberMediaTone` (Compose): монтаж подписывает, демонтаж отменяет, приложение не создаёт `Task` и не читает тон повторно.

### Swift

```swift
WithMediaTone(source: item, band: 0.15) { tone, error in
    Bar(color: tone?.top.color)   // tone is nil until the core knows one
}
```

### Kotlin

```kotlin
val tone by rememberMediaTone(item, 0.15f)
Bar(color = tone.tone?.top?.color)  // tone.error holds the failure of the watch
```

Правила обёрток:

- Первое значение приходит после установки наблюдения: `nil`, BlurHash или пиксели. Изменение во время первого чтения не теряется. Несколько изменений подряд объединяются в одно чтение.
- Источник `nil` снимает подписку и очищает значение. Другой источник заменяет подписку и очищает значение прежнего. Другой `band` заменяет запрос, пока значение остаётся, и не запускает анализ пикселей.
- Платформа не хранит значения по URL. Хранит их ядро в ограниченной таблице и в записи кэша.

### Низкоуровневая подписка

Потребитель вне интерфейса вызывает `subscribeTone(source, band, listener)`. Слушатель получает `Option<MediaTone>`: сначала после установки наблюдения, затем после каждого изменения. Храните подписку, пока нужны обновления. Отмена снимает слушателя и удержанное чтение.

## What you do not do

- Приложение не декодирует изображения для тона. Перед подготовкой платформа спрашивает `tonePixelsNeeded`, затем уменьшает изображение до 64 пикселей по большей стороне.
- Без спроса и при готовом пиксельном тоне подготовок нет. Ядро допускает только одну подготовку на ключ одновременно.
- Ошибка освобождает запрос. Уход последнего подписчика отменяет подготовку, если нет незавершённого запроса `tone`. Повторный спрос запускает её снова.
- You do not store the tone yourself. To keep it with a row of your database, write it into a local field of that row (see the local fields section of the DBlayer models document).
- You do not run the analysis twice for one source: the first pixels of a source win, later ones are dropped.

## Изменения API

`submitTonePixels` удалён. Библиотека устанавливает один `TonePreparation` через `setTonePreparation` при открытии UI. Метод `prepare(source)` возвращает `TonePixels` и не хранит пиксели между вызовами.

Swift: `subscribeTone(url:listener:)` заменён на `subscribeTone(source:band:listener:)`. Kotlin: `subscribeTone(url, listener)` заменён на `subscribeTone(source, band, listener)`. `ToneListener.changed` получает `MediaTone?` вместо `ToneSource`. Передавайте `MediaItem` с заголовками вместо URL. Ручная связка `subscribeTone` с повторным `tone()` заменена на `WithMediaTone` и `rememberMediaTone`.

Сигнатуры `tone(source:band:)` и `tone(source, band)` сохранены. При отсутствии пиксельного результата запрос запускает подготовку. Тестовые реализации `MediaPlaces` реализуют `setTonePreparation`, `tonePixelsNeeded` и `playUrl`.
