use crate::api_types::{UiClientMessage, UiCommand, UiEvent};
use axum::{
    extract::{ws::rejection::WebSocketUpgradeRejection, ws::Message, ws::WebSocket, ws::WebSocketUpgrade, Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use futures::{SinkExt, StreamExt};
use serde_json::json;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, mpsc::UnboundedSender, RwLock};
use serde::Serialize;
use serde::Deserialize;

// Коротко про этот файл:
// 1) Поднимает HTTP/WS шлюз для мобильного клиента.
// 2) Проксирует команды из UI во внутренний app-loop.
// 3) Ведёт реестр username и проверяет уникальность/валидность.

// Жёсткие ограничения на формат ника.
const USERNAME_MIN_LEN: usize = 3;
const USERNAME_MAX_LEN: usize = 24;
// TTL для "наблюдавшихся" username: если давно не видели,
// освобождаем запись как потенциально устаревшую.
const USERNAME_TTL: Duration = Duration::from_secs(120);

#[derive(Debug, Serialize)]
struct UsernameCheckResponse {
    username: String,
    available: bool,
    message: String,
}

#[derive(Debug, Deserialize)]
struct UsernameCheckQuery {
    current: Option<String>,
}

#[derive(Clone)]
struct GatewayState {
    // Локальный peer id (для /peer и приветственного system-события).
    peer_id: String,
    // Канал команд в основной app-loop.
    command_tx: UnboundedSender<UiCommand>,
    // Broadcast событий всем подключенным WS-клиентам.
    events_tx: broadcast::Sender<UiEvent>,
    // Реестр занятых/наблюдавшихся username.
    known_usernames: Arc<RwLock<HashMap<String, Instant>>>,
}

pub async fn run_gateway(
    peer_id: String,
    command_tx: UnboundedSender<UiCommand>,
    events_tx: broadcast::Sender<UiEvent>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Глобальная карта известных username для проверки уникальности.
    let known_usernames: Arc<RwLock<HashMap<String, Instant>>> = Arc::new(RwLock::new(HashMap::new()));

    // Отдельная фоновая задача: слушаем события UsernameObserved
    // и поддерживаем реестр имён в актуальном состоянии.
    let mut events_rx_for_registry = events_tx.subscribe();
    let known_usernames_for_registry = known_usernames.clone();
    tokio::spawn(async move {
        while let Ok(event) = events_rx_for_registry.recv().await {
            if let UiEvent::UsernameObserved { username } = event {
                let normalized = normalize_username(&username);
                if !normalized.is_empty() {
                    known_usernames_for_registry
                        .write()
                        .await
                        .insert(normalized, Instant::now());
                }
            }
        }
    });

    let state = GatewayState {
        peer_id,
        command_tx,
        events_tx,
        known_usernames,
    };

    let router = Router::new()
        // Простые служебные эндпоинты.
        .route("/health", get(health_handler))
        .route("/peer", get(peer_handler))
        // HTTP-проверка ника (используется мобильным UI для live-check).
        .route("/check-username/{username}", get(check_username_handler))
        // Основной WS канал для realtime обмена.
        .route("/ws", get(ws_handler))
        .with_state(state);

    let addr: SocketAddr = "0.0.0.0:8080".parse()?;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    println!("📱 UI gateway запущен на http://{}", addr);
    axum::serve(listener, router).await?;
    Ok(())
}

async fn health_handler() -> Json<serde_json::Value> {
    Json(json!({ "ok": true }))
}

async fn peer_handler(State(state): State<GatewayState>) -> Json<serde_json::Value> {
    Json(json!({ "peer_id": state.peer_id }))
}

async fn check_username_handler(
    Path(username): Path<String>,
    Query(query): Query<UsernameCheckQuery>,
    State(state): State<GatewayState>,
) -> Json<UsernameCheckResponse> {
    // Нормализуем оба username (requested/current), чтобы сравнение было корректным.
    let requested = normalize_username(&username);
    let current = query.current.as_deref().map(normalize_username);
    let status = build_username_status(&requested, &state, current.as_deref(), false).await;

    match status {
        UiEvent::UsernameStatus {
            username,
            available,
            message,
            ..
        } => Json(UsernameCheckResponse {
            username,
            available,
            message,
        }),
        _ => Json(UsernameCheckResponse {
            username: requested,
            available: false,
            message: "Внутренняя ошибка проверки".to_owned(),
        }),
    }
}

async fn ws_handler(
    ws: Result<WebSocketUpgrade, WebSocketUpgradeRejection>,
    State(state): State<GatewayState>,
) -> impl IntoResponse {
    match ws {
        // Успешный WS upgrade.
        Ok(ws) => ws.on_upgrade(move |socket| handle_ws_client(socket, state)).into_response(),
        // Если клиент пришёл простым HTTP GET без upgrade-заголовков.
        Err(_) => (
            StatusCode::BAD_REQUEST,
            "WebSocket upgrade required. Подключайтесь через ws://<host>:8080/ws, а не обычным HTTP GET.",
        )
            .into_response(),
    }
}

async fn handle_ws_client(socket: WebSocket, state: GatewayState) {
    let (mut sender, mut receiver) = socket.split();
    let mut events_rx = state.events_tx.subscribe();
    // Персональное состояние клиента (какой ник он сейчас выбрал).
    let mut current_username: Option<String> = None;

    let hello = UiEvent::System {
        message: format!("connected_to_peer:{}", state.peer_id),
    };
    let hello_json = match serde_json::to_string(&hello) {
        Ok(text) => text,
        Err(_) => return,
    };

    if sender.send(Message::Text(hello_json.into())).await.is_err() {
        return;
    }

    let mut send_task_sender = sender;
    let send_task = tokio::spawn(async move {
        // Задача на отправку: перекидываем broadcast-события в конкретный WS.
        while let Ok(event) = events_rx.recv().await {
            let text = match serde_json::to_string(&event) {
                Ok(value) => value,
                Err(_) => continue,
            };
            if send_task_sender.send(Message::Text(text.into())).await.is_err() {
                break;
            }
        }
    });

    while let Some(Ok(message)) = receiver.next().await {
        match message {
            Message::Text(text) => {
                // Сначала пробуем JSON-протокол UI.
                if let Ok(parsed) = serde_json::from_str::<UiClientMessage>(&text) {
                    match parsed {
                        UiClientMessage::SendMessage { text } => {
                            let _ = state.command_tx.send(UiCommand::SendMessage {
                                text,
                                username: current_username.clone(),
                            });
                        }
                        UiClientMessage::CheckUsername { username } => {
                            // Проверяем ник без применения.
                            let requested = normalize_username(&username);
                            let status = build_username_status(
                                &requested,
                                &state,
                                current_username.as_deref(),
                                false,
                            )
                            .await;
                            let _ = state.events_tx.send(status);
                        }
                        UiClientMessage::SetUsername { username } => {
                            // Проверяем ник и, если свободен, применяем к текущему WS-клиенту.
                            let requested = normalize_username(&username);
                            let status = build_username_status(
                                &requested,
                                &state,
                                current_username.as_deref(),
                                false,
                            )
                            .await;

                            if matches!(status, UiEvent::UsernameStatus { available: true, .. }) {
                                // Помечаем ник занятым и публикуем наблюдение.
                                state
                                    .known_usernames
                                    .write()
                                    .await
                                    .insert(requested.clone(), Instant::now());
                                current_username = Some(requested.clone());
                                let _ = state.events_tx.send(UiEvent::UsernameObserved {
                                    username: requested.clone(),
                                });
                                let _ = state.events_tx.send(UiEvent::UsernameStatus {
                                    username: requested,
                                    available: true,
                                    applied: true,
                                    message: "Ник свободен и сохранён".to_owned(),
                                });
                            } else {
                                let _ = state.events_tx.send(status);
                            }
                        }
                        UiClientMessage::Ping => {}
                    }
                } else {
                    // Backward compatibility: если пришёл просто текстом, трактуем как chat.
                    let plain = text.trim().to_owned();
                    if !plain.is_empty() {
                        let _ = state.command_tx.send(UiCommand::SendMessage {
                            text: plain,
                            username: current_username.clone(),
                        });
                    }
                }
            }
            Message::Close(_) => break,
            _ => {}
        }
    }

    send_task.abort();
}

fn normalize_username(value: &str) -> String {
    // Вся система ника работает в lower-case + trim,
    // чтобы не было дублей вида "User" и " user ".
    value.trim().to_lowercase()
}

fn validate_username_format(value: &str) -> Result<(), String> {
    if value.is_empty() {
        return Err("Ник не должен быть пустым".to_owned());
    }

    if value.len() < USERNAME_MIN_LEN {
        return Err(format!("Минимум {} символа", USERNAME_MIN_LEN));
    }

    if value.len() > USERNAME_MAX_LEN {
        return Err(format!("Максимум {} символа", USERNAME_MAX_LEN));
    }

    if !value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' || ch == '.')
    {
        return Err("Разрешены только a-z, 0-9, '.', '_' и '-'".to_owned());
    }

    Ok(())
}

async fn build_username_status(
    requested: &str,
    state: &GatewayState,
    current_username: Option<&str>,
    applied: bool,
) -> UiEvent {
    // Этап 1: базовая валидация формата.
    if let Err(message) = validate_username_format(requested) {
        return UiEvent::UsernameStatus {
            username: requested.to_owned(),
            available: false,
            applied,
            message,
        };
    }

    let available = {
        let mut known = state.known_usernames.write().await;
        let now = Instant::now();
        // Этап 2: сборка мусора по TTL, чтобы старые записи не блокировали ник навсегда.
        known.retain(|_, seen_at| now.duration_since(*seen_at) <= USERNAME_TTL);

        // Этап 3: проверка занятости.
        // Если пользователь проверяет свой текущий ник — разрешаем.
        if let Some(current) = current_username {
            requested == current || !known.contains_key(requested)
        } else {
            !known.contains_key(requested)
        }
    };

    if available {
        UiEvent::UsernameStatus {
            username: requested.to_owned(),
            available: true,
            applied,
            message: if applied {
                "Ник свободен и сохранён".to_owned()
            } else {
                "Ник свободен".to_owned()
            },
        }
    } else {
        UiEvent::UsernameStatus {
            username: requested.to_owned(),
            available: false,
            applied,
            message: "Этот ник уже занят".to_owned(),
        }
    }
}
