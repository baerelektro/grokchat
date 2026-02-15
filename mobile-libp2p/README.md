# mobile-libp2p (план + скелет)

Цель: каждый мобильный пользователь = отдельный libp2p-узел (свой `PeerId` / DHT-участник), без зависимости от gateway как «заменителя пира».

## Что уже есть в этом скелете

- Документ архитектуры и этапов внедрения
- Rust core crate (`mobile-libp2p/core`) с базовыми типами команд/событий
- Контракт для bridge-слоя (Android/iOS)

## Этапы реализации

1. Core runtime (Rust): старт узла, keypair, события сети
2. Gossipsub: join topic, send/recv message
3. Kademlia: bootstrap + provider lookup
4. Blob-store: chunk API (put/get)
5. Bridge: UniFFI/JNI/Swift bindings
6. Mobile app: React Native (native module) или Flutter

## Почему не Expo Go

`Expo Go` не запускает ваш произвольный Rust native core с libp2p.
Нужен prebuild/dev client (или bare RN), где подключается нативный модуль.

## Что делать дальше

- Реализовать `start_node()` и сетевой loop в `core/src/lib.rs`
- Добавить `tokio + libp2p` зависимости в `core/Cargo.toml`
- Выбрать bridge:
  - RN: TurboModule + FFI
  - Flutter: rust_bridge
