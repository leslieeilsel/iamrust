use std::{sync::Arc, time::Duration};

use futures_util::{SinkExt as _, StreamExt as _};
use iamrust_domain::{ConversationId, SyncEvent, UserId};
use iamrust_protocol::{CallSignal, ClientFrame, ServerFrame, WS_PROTOCOL_VERSION};
use rand::Rng as _;
use tokio::{
    runtime::Runtime,
    sync::mpsc,
    time::{Instant, MissedTickBehavior},
};
use tokio_tungstenite::{
    connect_async_with_config,
    tungstenite::{Message as WsMessage, protocol::WebSocketConfig},
};
use uuid::Uuid;

use crate::api::ApiClient;

const EVENT_BUFFER: usize = 512;
const COMMAND_BUFFER: usize = 64;
const MAX_WEBSOCKET_MESSAGE_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    Connecting,
    Syncing,
    Online,
    Offline,
    Failed,
}

impl ConnectionState {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Connecting => "连接中",
            Self::Syncing => "同步中",
            Self::Online => "已连接",
            Self::Offline => "离线",
            Self::Failed => "同步失败",
        }
    }
}

#[derive(Debug, Clone)]
pub enum RealtimeEvent {
    State(ConnectionState),
    Sync(SyncEvent),
    Typing {
        conversation_id: ConversationId,
        user_id: UserId,
        active: bool,
    },
    Call {
        conversation_id: ConversationId,
        call_id: Uuid,
        from_user_id: UserId,
        signal: CallSignal,
    },
    Error(String),
}

#[derive(Debug)]
enum RealtimeCommand {
    Typing {
        conversation_id: ConversationId,
        active: bool,
    },
    Call {
        conversation_id: ConversationId,
        call_id: Uuid,
        target_user_id: Option<UserId>,
        signal: CallSignal,
    },
    Stop,
}

#[derive(Debug, Clone)]
pub struct RealtimeHandle {
    commands: mpsc::Sender<RealtimeCommand>,
}

impl RealtimeHandle {
    pub fn send_typing(&self, conversation_id: ConversationId, active: bool) {
        let _ = self.commands.try_send(RealtimeCommand::Typing {
            conversation_id,
            active,
        });
    }

    pub fn send_call(
        &self,
        conversation_id: ConversationId,
        call_id: Uuid,
        target_user_id: Option<UserId>,
        signal: CallSignal,
    ) {
        let _ = self.commands.try_send(RealtimeCommand::Call {
            conversation_id,
            call_id,
            target_user_id,
            signal,
        });
    }

    pub fn stop(&self) {
        let _ = self.commands.try_send(RealtimeCommand::Stop);
    }
}

pub fn spawn(
    runtime: &Arc<Runtime>,
    api: Arc<ApiClient>,
    cursor: u64,
) -> (RealtimeHandle, mpsc::Receiver<RealtimeEvent>) {
    let (command_tx, command_rx) = mpsc::channel(COMMAND_BUFFER);
    let (event_tx, event_rx) = mpsc::channel(EVENT_BUFFER);
    runtime.spawn(run(api, cursor, command_rx, event_tx));
    (
        RealtimeHandle {
            commands: command_tx,
        },
        event_rx,
    )
}

async fn run(
    api: Arc<ApiClient>,
    mut cursor: u64,
    mut commands: mpsc::Receiver<RealtimeCommand>,
    events: mpsc::Sender<RealtimeEvent>,
) {
    let mut retry = 0_u32;
    loop {
        emit_state(&events, ConnectionState::Connecting).await;
        let mut became_online = false;
        match connect_once(
            api.clone(),
            &mut cursor,
            &mut commands,
            events.clone(),
            &mut became_online,
        )
        .await
        {
            Ok(Disconnect::Stopped) => return,
            Ok(Disconnect::Reconnect) => {}
            Err(error) => {
                let fatal = error == "实时协议版本不兼容";
                let _ = events.send(RealtimeEvent::Error(error)).await;
                if fatal {
                    emit_state(&events, ConnectionState::Failed).await;
                    return;
                }
            }
        }
        if became_online {
            retry = 0;
        }
        emit_state(&events, ConnectionState::Offline).await;
        let delay = reconnect_delay(retry);
        retry = retry.saturating_add(1);
        tokio::select! {
            () = tokio::time::sleep(delay) => {}
            command = commands.recv() => {
                if command.is_none_or(|command| matches!(command, RealtimeCommand::Stop)) {
                    return;
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Disconnect {
    Stopped,
    Reconnect,
}

async fn connect_once(
    api: Arc<ApiClient>,
    cursor: &mut u64,
    commands: &mut mpsc::Receiver<RealtimeCommand>,
    events: mpsc::Sender<RealtimeEvent>,
    became_online: &mut bool,
) -> Result<Disconnect, String> {
    let session_api = api.clone();
    let last_cursor = *cursor;
    let session = tokio::task::spawn_blocking(move || session_api.websocket_session(last_cursor))
        .await
        .map_err(|_| "实时连接任务异常终止".to_owned())?
        .map_err(|error| error.user_message())?;
    let config = WebSocketConfig::default()
        .max_message_size(Some(MAX_WEBSOCKET_MESSAGE_BYTES))
        .max_frame_size(Some(MAX_WEBSOCKET_MESSAGE_BYTES));
    let (mut socket, _) = connect_async_with_config(session.url.as_str(), Some(config), true)
        .await
        .map_err(|_| "无法建立实时连接".to_owned())?;
    send_frame(&mut socket, &session.hello).await?;

    let mut heartbeat = tokio::time::interval(Duration::from_secs(20));
    heartbeat.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut last_pong = Instant::now();
    loop {
        tokio::select! {
            _ = heartbeat.tick() => {
                if last_pong.elapsed() > Duration::from_secs(45) {
                    return Err("实时连接心跳超时".to_owned());
                }
                send_frame(&mut socket, &ClientFrame::Ping { nonce: Uuid::new_v4() }).await?;
            }
            command = commands.recv() => {
                let Some(command) = command else { return Ok(Disconnect::Stopped) };
                match command {
                    RealtimeCommand::Typing { conversation_id, active } => {
                        send_frame(&mut socket, &ClientFrame::Typing { conversation_id, active }).await?;
                    }
                    RealtimeCommand::Call {
                        conversation_id,
                        call_id,
                        target_user_id,
                        signal,
                    } => {
                        send_frame(
                            &mut socket,
                            &ClientFrame::CallSignal {
                                conversation_id,
                                call_id,
                                target_user_id,
                                signal,
                            },
                        )
                        .await?;
                    }
                    RealtimeCommand::Stop => {
                        let _ = socket.close(None).await;
                        return Ok(Disconnect::Stopped);
                    }
                }
            }
            incoming = socket.next() => {
                let Some(incoming) = incoming else { return Ok(Disconnect::Reconnect) };
                match incoming.map_err(|_| "实时连接读取失败".to_owned())? {
                    WsMessage::Text(text) => {
                        let frame = serde_json::from_str::<ServerFrame>(&text)
                            .map_err(|_| "实时协议响应无效".to_owned())?;
                        let welcome = matches!(&frame, ServerFrame::Welcome { .. });
                        if let Some(disconnect) = handle_server_frame(
                            frame,
                            api.clone(),
                            cursor,
                            &events,
                            &mut last_pong,
                        )
                        .await?
                        {
                            return Ok(disconnect);
                        }
                        if welcome {
                            *became_online = true;
                        }
                    }
                    WsMessage::Ping(payload) => socket
                        .send(WsMessage::Pong(payload))
                        .await
                        .map_err(|_| "实时连接写入失败".to_owned())?,
                    WsMessage::Pong(_) => last_pong = Instant::now(),
                    WsMessage::Close(_) => return Ok(Disconnect::Reconnect),
                    WsMessage::Binary(_) | WsMessage::Frame(_) => {
                        return Err("实时协议收到不支持的帧".to_owned());
                    }
                }
            }
        }
    }
}

async fn handle_server_frame(
    frame: ServerFrame,
    api: Arc<ApiClient>,
    cursor: &mut u64,
    events: &mpsc::Sender<RealtimeEvent>,
    last_pong: &mut Instant,
) -> Result<Option<Disconnect>, String> {
    match frame {
        ServerFrame::Welcome {
            protocol_version,
            latest_cursor,
            ..
        } => {
            if protocol_version != WS_PROTOCOL_VERSION {
                emit_state(events, ConnectionState::Failed).await;
                return Err("实时协议版本不兼容".to_owned());
            }
            if latest_cursor > *cursor {
                emit_state(events, ConnectionState::Syncing).await;
                *cursor = catch_up(api, *cursor, events).await?;
            }
            emit_state(events, ConnectionState::Online).await;
        }
        ServerFrame::Pong { .. } => *last_pong = Instant::now(),
        ServerFrame::Event { event } if event.cursor > *cursor => {
            *cursor = event.cursor;
            if events.send(RealtimeEvent::Sync(event)).await.is_err() {
                return Ok(Some(Disconnect::Stopped));
            }
        }
        ServerFrame::Typing {
            conversation_id,
            user_id,
            active,
            ..
        } => {
            let _ = events
                .send(RealtimeEvent::Typing {
                    conversation_id,
                    user_id,
                    active,
                })
                .await;
        }
        ServerFrame::CallSignal {
            conversation_id,
            call_id,
            from_user_id,
            signal,
        } => {
            let _ = events
                .send(RealtimeEvent::Call {
                    conversation_id,
                    call_id,
                    from_user_id,
                    signal,
                })
                .await;
        }
        ServerFrame::Error { error, .. } => {
            let _ = events.send(RealtimeEvent::Error(error.message_key)).await;
        }
        ServerFrame::Close { reason, .. } => {
            let _ = events.send(RealtimeEvent::Error(reason)).await;
            return Ok(Some(Disconnect::Reconnect));
        }
        ServerFrame::Ack { .. } | ServerFrame::Event { .. } => {}
    }
    Ok(None)
}

async fn catch_up(
    api: Arc<ApiClient>,
    mut cursor: u64,
    events: &mpsc::Sender<RealtimeEvent>,
) -> Result<u64, String> {
    loop {
        let sync_api = api.clone();
        let response = tokio::task::spawn_blocking(move || sync_api.sync(cursor, 200))
            .await
            .map_err(|_| "离线同步任务异常终止".to_owned())?
            .map_err(|error| error.user_message())?;
        for event in response.events {
            cursor = cursor.max(event.cursor);
            if events.send(RealtimeEvent::Sync(event)).await.is_err() {
                return Err("客户端已停止".to_owned());
            }
        }
        cursor = cursor.max(response.next_cursor);
        if !response.has_more {
            return Ok(cursor);
        }
    }
}

async fn send_frame<S>(socket: &mut S, frame: &ClientFrame) -> Result<(), String>
where
    S: futures_util::Sink<WsMessage> + Unpin,
{
    let payload = serde_json::to_string(frame).map_err(|_| "实时请求序列化失败".to_owned())?;
    socket
        .send(WsMessage::Text(payload.into()))
        .await
        .map_err(|_| "实时连接写入失败".to_owned())
}

async fn emit_state(events: &mpsc::Sender<RealtimeEvent>, state: ConnectionState) {
    let _ = events.send(RealtimeEvent::State(state)).await;
}

fn reconnect_delay(retry: u32) -> Duration {
    let base = 800_u64.saturating_mul(1_u64 << retry.min(6)).min(30_000);
    let low = base.saturating_mul(3) / 4;
    let high = base.saturating_mul(5) / 4;
    Duration::from_millis(rand::rng().random_range(low..=high))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconnect_delay_is_jittered_and_capped() {
        for retry in 0..20 {
            let delay = reconnect_delay(retry);
            assert!(delay >= Duration::from_millis(600));
            assert!(delay <= Duration::from_millis(37_500));
        }
    }
}
