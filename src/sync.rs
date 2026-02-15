use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SyncEnvelope {
    Chat {
        username: Option<String>,
        text: String,
    },
    Presence {
        username: Option<String>,
        status: String,
        ts: u64,
    },
    Profile {
        username: String,
    },
}

fn now_ts() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub fn encode_chat(text: &str, username: Option<&str>) -> Vec<u8> {
    let envelope = SyncEnvelope::Chat {
        username: username.map(ToOwned::to_owned),
        text: text.to_owned(),
    };

    serde_json::to_vec(&envelope).unwrap_or_else(|_| text.as_bytes().to_vec())
}

pub fn encode_presence(username: Option<&str>, status: &str) -> Vec<u8> {
    let envelope = SyncEnvelope::Presence {
        username: username.map(ToOwned::to_owned),
        status: status.to_owned(),
        ts: now_ts(),
    };

    serde_json::to_vec(&envelope).unwrap_or_else(|_| status.as_bytes().to_vec())
}

pub fn encode_profile(username: &str) -> Vec<u8> {
    let envelope = SyncEnvelope::Profile {
        username: username.to_owned(),
    };

    serde_json::to_vec(&envelope).unwrap_or_else(|_| username.as_bytes().to_vec())
}

pub fn decode(payload: &[u8]) -> Option<SyncEnvelope> {
    serde_json::from_slice::<SyncEnvelope>(payload).ok()
}
