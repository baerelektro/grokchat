use crate::api_types::{UiClientMessage, UiCommand, UiEvent};
use axum::{
    extract::{ws::rejection::WebSocketUpgradeRejection, ws::Message, ws::WebSocket, ws::WebSocketUpgrade, State},
    http::StatusCode,
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use futures::{SinkExt, StreamExt};
use serde_json::json;
use std::net::SocketAddr;
use tokio::sync::{broadcast, mpsc::UnboundedSender};

#[derive(Clone)]
struct GatewayState {
    peer_id: String,
    command_tx: UnboundedSender<UiCommand>,
    events_tx: broadcast::Sender<UiEvent>,
}

pub async fn run_gateway(
    peer_id: String,
    command_tx: UnboundedSender<UiCommand>,
    events_tx: broadcast::Sender<UiEvent>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let state = GatewayState {
        peer_id,
        command_tx,
        events_tx,
    };

    let router = Router::new()
        .route("/health", get(health_handler))
        .route("/peer", get(peer_handler))
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

async fn ws_handler(
    ws: Result<WebSocketUpgrade, WebSocketUpgradeRejection>,
    State(state): State<GatewayState>,
) -> impl IntoResponse {
    match ws {
        Ok(ws) => ws.on_upgrade(move |socket| handle_ws_client(socket, state)).into_response(),
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
                if let Ok(parsed) = serde_json::from_str::<UiClientMessage>(&text) {
                    match parsed {
                        UiClientMessage::SendMessage { text } => {
                            let _ = state.command_tx.send(UiCommand::SendMessage { text });
                        }
                        UiClientMessage::Ping => {}
                    }
                } else {
                    let plain = text.trim().to_owned();
                    if !plain.is_empty() {
                        let _ = state.command_tx.send(UiCommand::SendMessage { text: plain });
                    }
                }
            }
            Message::Close(_) => break,
            _ => {}
        }
    }

    send_task.abort();
}
