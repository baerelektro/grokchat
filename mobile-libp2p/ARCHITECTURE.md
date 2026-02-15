# Архитектура mobile libp2p

## 1. Компоненты

- `mobile-core` (Rust)
  - libp2p transport
  - gossipsub
  - kademlia
  - (позже) request/response для chunks
- `mobile-bridge`
  - адаптер между Rust и нативным UI runtime
- `mobile-ui`
  - чат, профиль, статус сети

## 2. Потоки данных

### От UI к сети

`UI -> BridgeCommand::SendChat -> core -> gossipsub.publish`

### От сети к UI

`libp2p event -> core -> CoreEvent::ChatMessage/Presence/Profile -> UI`

## 3. Идентичность

- На первом запуске генерируется keypair
- Keypair сохраняется в secure storage платформы
- При следующем запуске восстанавливается тот же PeerId

## 4. Синхронизация между нодами

- Канал сообщений: gossipsub topic (`grok-chat`)
- Presence: периодический heartbeat envelope
- Profile: отправка profile envelope на старт/изменение

## 5. DHT (Kademlia)

- bootstrap peers в настройках
- `GET_PROVIDERS/START_PROVIDING` для blob keys
- хранение известных peers в локальном state

## 6. Файлы

- chunking (1 MiB)
- hash (`blake3`)
- manifest c metadata + chunk hashes
- transport chunks через request/response

## 7. Безопасность

- Noise transport
- подпись envelope источником
- (опционально) E2EE payload для приватных чатов
