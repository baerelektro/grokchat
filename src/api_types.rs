use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum UiEvent {
    ChatMessage { from: String, text: String },
    System { message: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum UiClientMessage {
    SendMessage { text: String },
    Ping,
}

#[derive(Debug, Clone)]
pub enum UiCommand {
    SendMessage { text: String },
}
