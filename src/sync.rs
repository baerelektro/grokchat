use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

// Коротко про этот файл:
// 1) Определяет единый wire-формат событий синхронизации между пирами.
// 2) Даёт encode/decode helper-функции для chat/presence/profile.
// 3) Держит сериализацию максимально простой (JSON), чтобы легко дебажить.

// Универсальный конверт (envelope) для событий синхронизации.
// Он уходит в gossipsub payload как JSON.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SyncEnvelope {
    // Чат-сообщение пользователя.
    Chat {
        username: Option<String>,
        text: String,
    },
    // Presence-событие: кто онлайн и в каком статусе.
    Presence {
        username: Option<String>,
        status: String,
        ts: u64,
    },
    // Профиль (минимум: username) для распространения по сети.
    Profile {
        username: String,
    },
}

// Текущий UNIX timestamp в секундах.
fn now_ts() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

// Кодирует чат в JSON envelope.
// Fallback: если JSON-сериализация внезапно не удалась, отправляем сырой текст.
pub fn encode_chat(text: &str, username: Option<&str>) -> Vec<u8> {
    let envelope = SyncEnvelope::Chat {
        username: username.map(ToOwned::to_owned),
        text: text.to_owned(),
    };

    serde_json::to_vec(&envelope).unwrap_or_else(|_| text.as_bytes().to_vec())
}

// Кодирует presence в JSON envelope.
pub fn encode_presence(username: Option<&str>, status: &str) -> Vec<u8> {
    let envelope = SyncEnvelope::Presence {
        username: username.map(ToOwned::to_owned),
        status: status.to_owned(),
        ts: now_ts(),
    };

    serde_json::to_vec(&envelope).unwrap_or_else(|_| status.as_bytes().to_vec())
}

// Кодирует профиль пользователя.
pub fn encode_profile(username: &str) -> Vec<u8> {
    let envelope = SyncEnvelope::Profile {
        username: username.to_owned(),
    };

    serde_json::to_vec(&envelope).unwrap_or_else(|_| username.as_bytes().to_vec())
}

// Пытается декодировать JSON payload в SyncEnvelope.
// Если payload не нашего формата — возвращает None.
pub fn decode(payload: &[u8]) -> Option<SyncEnvelope> {
    serde_json::from_slice::<SyncEnvelope>(payload).ok()
}
