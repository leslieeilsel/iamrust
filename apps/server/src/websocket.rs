use std::time::{Duration, Instant};

use axum::extract::ws::{Message as WsMessage, WebSocket, close_code};
use chrono::Utc;
use futures_util::{SinkExt, StreamExt};
use iamrust_application::ChatService;
use iamrust_domain::UserId;
use iamrust_protocol::{ApiError, ClientFrame, ErrorCode, ServerFrame, WS_PROTOCOL_VERSION};
use tokio::time::MissedTickBehavior;
use uuid::Uuid;

pub async fn serve(socket: WebSocket, service: ChatService, user_id: UserId) {
    if service
        .connection_opened(user_id, Utc::now())
        .await
        .is_err()
    {
        return;
    }
    let (mut sender, mut receiver) = socket.split();
    let mut events = service.subscribe();
    let mut typing = service.subscribe_typing();
    let mut calls = service.subscribe_calls();
    if send_welcome(&mut sender, &service, user_id).await.is_err() {
        let _ = service.connection_closed(user_id, Utc::now()).await;
        return;
    }

    let mut heartbeat = tokio::time::interval(Duration::from_secs(25));
    heartbeat.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut last_activity = Instant::now();

    loop {
        tokio::select! {
            _ = heartbeat.tick() => {
                if heartbeat_tick(&mut sender, last_activity).await.is_err() {
                    break;
                }
            }
            event = events.recv() => {
                match event {
                    Ok((recipients, event)) if recipients.contains(&user_id) => {
                        if send_frame(&mut sender, &ServerFrame::Event { event }).await.is_err() {
                            break;
                        }
                    }
                    Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
            signal = typing.recv() => {
                match signal {
                    Ok((recipients, signal)) if recipients.contains(&user_id) => {
                        if send_frame(&mut sender, &ServerFrame::Typing {
                            conversation_id: signal.conversation_id,
                            user_id: signal.user_id,
                            active: signal.active,
                            expires_at: signal.expires_at,
                        }).await.is_err() {
                            break;
                        }
                    }
                    Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
            delivery = calls.recv() => {
                match delivery {
                    Ok((recipients, delivery)) if recipients.contains(&user_id) => {
                        if send_frame(&mut sender, &ServerFrame::CallSignal {
                            conversation_id: delivery.conversation_id,
                            call_id: delivery.call_id,
                            from_user_id: delivery.from_user_id,
                            signal: delivery.signal,
                        }).await.is_err() {
                            break;
                        }
                    }
                    Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
            incoming = receiver.next() => {
                let Some(incoming) = incoming else { break };
                match incoming {
                    Ok(WsMessage::Text(text)) => {
                        last_activity = Instant::now();
                        let frame = serde_json::from_str::<ClientFrame>(&text);
                        if let Err(error) = handle_frame(frame, &service, user_id, &mut sender).await
                            && send_frame(&mut sender, &error).await.is_err()
                        {
                            break;
                        }
                    }
                    Ok(WsMessage::Pong(_) | WsMessage::Ping(_)) => {
                        last_activity = Instant::now();
                    }
                    Ok(WsMessage::Close(_)) | Err(_) => break,
                    Ok(WsMessage::Binary(_)) => {
                        let _ = send_frame(&mut sender, &protocol_error(None, "binary_not_supported")).await;
                    }
                }
            }
        }
    }
    let _ = service.connection_closed(user_id, Utc::now()).await;
}

async fn heartbeat_tick<S>(sender: &mut S, last_activity: Instant) -> Result<(), ()>
where
    S: futures_util::Sink<WsMessage> + Unpin,
{
    if last_activity.elapsed() > Duration::from_secs(75) {
        let _ = sender
            .send(WsMessage::Close(Some(axum::extract::ws::CloseFrame {
                code: close_code::AWAY,
                reason: "heartbeat timeout".into(),
            })))
            .await;
        return Err(());
    }
    sender
        .send(WsMessage::Ping(Vec::new().into()))
        .await
        .map_err(|_| ())
}

async fn send_welcome<S>(sender: &mut S, service: &ChatService, user_id: UserId) -> Result<(), ()>
where
    S: futures_util::Sink<WsMessage> + Unpin,
{
    send_frame(
        sender,
        &ServerFrame::Welcome {
            protocol_version: WS_PROTOCOL_VERSION,
            user_id,
            server_time: Utc::now(),
            latest_cursor: service.latest_cursor().await,
        },
    )
    .await
}

async fn handle_frame<S>(
    frame: Result<ClientFrame, serde_json::Error>,
    service: &ChatService,
    user_id: UserId,
    sender: &mut S,
) -> Result<(), ServerFrame>
where
    S: futures_util::Sink<WsMessage> + Unpin,
{
    let frame = frame.map_err(|_| protocol_error(None, "invalid_frame"))?;
    match frame {
        ClientFrame::Hello {
            protocol_version, ..
        } if protocol_version != WS_PROTOCOL_VERSION => {
            Err(protocol_error(None, "protocol_mismatch"))
        }
        ClientFrame::Hello { .. } => Ok(()),
        ClientFrame::Typing {
            conversation_id,
            active,
        } => service
            .publish_typing(user_id, conversation_id, active, Utc::now())
            .await
            .map_err(|_| protocol_error(None, "typing_forbidden")),
        ClientFrame::CallSignal {
            conversation_id,
            call_id,
            target_user_id,
            signal,
        } => service
            .publish_call_signal(
                user_id,
                conversation_id,
                call_id,
                target_user_id,
                signal,
                Utc::now(),
            )
            .await
            .map_err(|_| protocol_error(None, "call_signal_forbidden")),
        ClientFrame::Ping { nonce } => send_frame(sender, &ServerFrame::Pong { nonce })
            .await
            .map_err(|()| protocol_error(None, "connection_closed")),
        ClientFrame::SendMessage {
            request_id,
            conversation_id,
            message,
        } => {
            let (_, ack) = service
                .send_message_request(user_id, conversation_id, *message, Utc::now())
                .await
                .map_err(|_| protocol_error(Some(request_id), "send_failed"))?;
            send_frame(sender, &ServerFrame::Ack { request_id, ack })
                .await
                .map_err(|()| protocol_error(Some(request_id), "connection_closed"))
        }
        ClientFrame::MarkRead {
            request_id,
            conversation_id,
            through_sequence,
        } => service
            .mark_read(user_id, conversation_id, through_sequence, Utc::now())
            .await
            .map_err(|_| protocol_error(Some(request_id), "mark_read_failed")),
    }
}

async fn send_frame<S>(sender: &mut S, frame: &ServerFrame) -> Result<(), ()>
where
    S: futures_util::Sink<WsMessage> + Unpin,
{
    let payload = serde_json::to_string(frame).map_err(|_| ())?;
    sender
        .send(WsMessage::Text(payload.into()))
        .await
        .map_err(|_| ())
}

fn protocol_error(request_id: Option<Uuid>, key: &str) -> ServerFrame {
    ServerFrame::Error {
        request_id,
        error: ApiError {
            code: if key == "protocol_mismatch" {
                ErrorCode::ProtocolMismatch
            } else {
                ErrorCode::Validation
            },
            message_key: format!("error.{key}"),
            field: None,
            correlation_id: Uuid::now_v7(),
            retryable: false,
        },
    }
}
