use serde::{Deserialize, Serialize};

// Коротко про этот файл:
// 1) Описывает контракты сообщений между backend и мобильным UI.
// 2) Разделяет направление событий: от сервера к UI и от UI к серверу.
// 3) Держит единый формат для сериализации/десериализации через serde.
//
// ВАЖНО:
// - `UiEvent` -> это то, что backend отправляет клиентам (WS broadcast).
// - `UiClientMessage` -> это то, что клиент присылает backend.
// - `UiCommand` -> внутренние команды внутри backend (не наружный JSON API).

// События, которые сервер отправляет в UI.
// Атрибут `#[serde(tag = "type")]` добавляет поле `type` в JSON,
// чтобы на клиенте можно было удобно делать switch по типу события.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum UiEvent {
    // Обычное чат-сообщение (отправитель + текст).
    ChatMessage { from: String, text: String },
    // Системная строка (служебный статус, сообщение об ошибке, и т.п.).
    System { message: String },
    // Наблюдение ника в сети: используем для регистра занятых username.
    UsernameObserved { username: String },
    // Ответ статуса ника (свободен/занят + применён ли).
    UsernameStatus {
        username: String,
        available: bool,
        applied: bool,
        message: String,
    },
}

// Сообщения, которые UI отправляет на backend по WebSocket.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum UiClientMessage {
    // Просьба отправить чат-сообщение.
    SendMessage { text: String },
    // Проверка доступности ника без сохранения.
    CheckUsername { username: String },
    // Сохранение ника (если backend подтвердит доступность).
    SetUsername { username: String },
    // Пинг для поддержания/проверки канала.
    Ping,
}

// Внутренний enum команд для app-цикла.
// Здесь специально минимум вариантов: чем проще внутренний протокол,
// тем легче сопровождать event-loop.
#[derive(Debug, Clone)]
pub enum UiCommand {
    SendMessage { text: String, username: Option<String> },
}
