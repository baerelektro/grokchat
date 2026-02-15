# Bridge contract (Android/iOS)

## Методы, которые вызывает UI

- `startNode(configJson: string)`
- `sendCommand(commandJson: string)`
- `stopNode()`

## События из Rust в UI

- `started { peer_id }`
- `chat_message { from, text }`
- `presence { from, status, ts }`
- `profile { from, username }`
- `error { message }`

## JSON примеры

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

- На iOS/Android bridge должен держать background thread для core loop.
- Все callback в UI-поток прокидывать через platform dispatcher.
- Ошибки сериализации/валидации всегда отсылать событием `error`.
