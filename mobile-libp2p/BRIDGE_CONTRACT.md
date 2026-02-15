# Bridge contract (Android/iOS)

Этот документ — формальный контракт между нативным UI-слоем и Rust core.
Главная цель: обе стороны должны одинаково понимать методы, события и JSON-схемы.

## Методы, которые вызывает UI

Ниже минимальный API жизненного цикла узла.

- `startNode(configJson: string)`
- `sendCommand(commandJson: string)`
- `stopNode()`

Смысл методов:
- `startNode` — поднимает core runtime и сеть;
- `sendCommand` — отправляет типизированную команду в уже запущенный core;
- `stopNode` — корректно завершает loop и освобождает ресурсы.

## События из Rust в UI

Это односторонний поток событий для отображения состояния и данных в интерфейсе.

- `started { peer_id }`
- `chat_message { from, text }`
- `presence { from, status, ts }`
- `profile { from, username }`
- `error { message }`

Рекомендация по обработке:
- `error` всегда показывать пользователю или писать в telemetry/log;
- `started` использовать как флаг «узел реально поднялся».

## JSON примеры

Примеры ниже считаются эталонными для сериализации/десериализации между слоями.

### start config

```json
{
  "username": "alice",
  "topic": "grok-chat",
  "bootstrap_peers": [
    "/ip4/192.168.1.10/tcp/4001/p2p/12D3Koo..."
  ]
}
```

### send chat command

```json
{
  "type": "send_chat",
  "text": "привет"
}
```

### update profile command

```json
{
  "type": "update_profile",
  "username": "alice-new"
}
```

## Примечания

Этот раздел про runtime-ограничения мобильных платформ.

- На iOS/Android bridge должен держать background thread для core loop.
- Все callback в UI-поток прокидывать через platform dispatcher.
- Ошибки сериализации/валидации всегда отсылать событием `error`.

Практически это защищает от типичных багов:
- блокировка UI из-за тяжёлой сетевой логики;
- краши из-за вызовов UI не из main-thread;
- «тихие» ошибки без обратной связи в интерфейсе.
