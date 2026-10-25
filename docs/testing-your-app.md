# Тесты приложения

Библиотека отдаёт тестам приложения подменный fetcher и снимок плеера. Оба подключаются только в тестовой цели приложения.

## Подключение

| Платформа | Где подключить | Что даёт |
| --------- | -------------- | -------- |
| Swift | Продукт `MediaLayerTesting` в тестовой цели | `TestFetcher` |
| Kotlin | `testImplementation(project(":testing"))` | `TestFetcher` (`com.noma4i.medialayer.testing`) |
| Swift | `MediaLayerUI` | `LoadedSlots.snapshot(for:)`, `PlayerSnapshot` |
| Kotlin | `kotlin/ui` | `MediaLayer.slotPlayers?.snapshot(key)`, `PlayerSnapshot` |

## TestFetcher

`TestFetcher` отдаёт заданные байты, файл или ассет по URL и честно отвечает на `Range`.

| Запрос | Ответ |
| ------ | ----- |
| Без `Range` | 200, `Content-Length`, `Accept-Ranges: bytes` |
| `Range` внутри тела | 206, `Content-Range: bytes s-e/всего`, конец включительный |
| Конец за пределом тела | 206, конец сжат до последнего байта |
| Начало за пределом тела | 416, `Content-Range: bytes */всего`, пустое тело |
| URL не задан | ошибка `Fetch("offline")` |

| Swift | Kotlin | Смысл |
| ----- | ------ | ----- |
| `serve(_:at:contentType:)` | `serve(url, data, contentType)` | байты |
| `serveFile(_:at:contentType:)` | `serveFile(url, file, contentType)`, `serveAsset(url, assets, name, contentType)` | файл, ассет приложения |
| `delay(_:for:)` | `delay(url, millis)` | задержка до ответа, отмена прерывает её |
| `fail(_:detail:)` | `fail(url, detail)` | ошибка `Fetch` для URL |
| `requests`, `requestCount(for:)` | `requests`, `requestCount(url)` | все запросы в порядке прихода |

## Снимок плеера

`PlayerSnapshot(key, isPlaying, isMuted, positionMs, rate)` читает состояние слота, который держит ключ. `isPlaying` это команда play без pause, не факт кадров. Нет слота у ключа: `nil`. Чтение не требует главного потока.

Swift: `LoadedSlots.of(places).snapshot(for: key)`. Kotlin: `media.slotPlayers?.snapshot(key)`.
