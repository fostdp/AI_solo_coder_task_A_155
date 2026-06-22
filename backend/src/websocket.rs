use crate::alerts::AlertManager;
use crate::models::WebSocketMessage;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::IntoResponse;
use futures_util::{SinkExt, StreamExt};
use std::sync::Arc;
use tracing::{debug, error, info};

pub async fn websocket_handler(
    ws: WebSocketUpgrade,
    State(alert_manager): State<Arc<AlertManager>>,
) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_socket(socket, alert_manager))
}

async fn handle_socket(socket: WebSocket, alert_manager: Arc<AlertManager>) {
    let (mut sender, mut receiver) = socket.split();
    let mut rx = alert_manager.sender().subscribe();

    info!("新WebSocket客户端已连接");

    let send_task = tokio::spawn(async move {
        while let Ok(msg) = rx.recv().await {
            let json = match serde_json::to_string(&msg) {
                Ok(j) => j,
                Err(e) => {
                    error!("序列化WebSocket消息失败: {}", e);
                    continue;
                }
            };

            if let Err(e) = sender.send(Message::Text(json.into())).await {
                debug!("WebSocket发送失败: {}", e);
                break;
            }
        }
    });

    let recv_task = tokio::spawn(async move {
        while let Some(Ok(msg)) = receiver.next().await {
            match msg {
                Message::Text(text) => {
                    debug!("收到WebSocket文本消息: {}", text);
                }
                Message::Close(_) => {
                    info!("WebSocket客户端请求关闭连接");
                    break;
                }
                _ => {}
            }
        }
    });

    tokio::select! {
        _ = send_task => {},
        _ = recv_task => {},
    }

    info!("WebSocket客户端已断开");
}

pub fn create_ws_message(message_type: &str, data: serde_json::Value) -> WebSocketMessage {
    WebSocketMessage {
        message_type: message_type.to_string(),
        data,
        timestamp: chrono::Utc::now(),
    }
}
