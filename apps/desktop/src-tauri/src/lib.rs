mod desktop;
mod local_store;
mod remote_api;

use std::fs;

use iamrust_domain::Message;
use iamrust_protocol::BootstrapResponse;
use local_store::{CacheStats, LocalStore, OutboxItem};
use tauri::{Manager, State};

#[tauri::command]
async fn cache_bootstrap(
    store: State<'_, LocalStore>,
    value: BootstrapResponse,
) -> Result<(), String> {
    store.cache_bootstrap(&value).await
}

#[tauri::command]
async fn load_cached_bootstrap(
    store: State<'_, LocalStore>,
) -> Result<Option<BootstrapResponse>, String> {
    store.load_bootstrap().await
}

#[tauri::command]
async fn cache_messages(
    store: State<'_, LocalStore>,
    messages: Vec<Message>,
) -> Result<(), String> {
    if messages.len() > 500 {
        return Err("too many messages".to_owned());
    }
    store.cache_messages(&messages).await
}

#[tauri::command]
async fn load_cached_messages(
    store: State<'_, LocalStore>,
    conversation_id: String,
) -> Result<Vec<Message>, String> {
    store.load_messages(&conversation_id).await
}

#[tauri::command]
async fn save_draft(
    store: State<'_, LocalStore>,
    conversation_id: String,
    body: String,
) -> Result<(), String> {
    store.save_draft(&conversation_id, &body).await
}

#[tauri::command]
async fn enqueue_outbox(
    store: State<'_, LocalStore>,
    client_message_id: String,
    conversation_id: String,
    payload_json: String,
) -> Result<(), String> {
    store
        .enqueue_outbox(&client_message_id, &conversation_id, &payload_json)
        .await
}

#[tauri::command]
async fn ready_outbox(store: State<'_, LocalStore>) -> Result<Vec<OutboxItem>, String> {
    store.ready_outbox().await
}

#[tauri::command]
async fn acknowledge_outbox(
    store: State<'_, LocalStore>,
    client_message_id: String,
) -> Result<(), String> {
    store.acknowledge_outbox(&client_message_id).await
}

#[tauri::command]
async fn clear_account_cache(store: State<'_, LocalStore>) -> Result<(), String> {
    store.clear_account_cache().await
}

#[tauri::command]
async fn cache_stats(
    app: tauri::AppHandle,
    store: State<'_, LocalStore>,
) -> Result<CacheStats, String> {
    let mut stats = store.cache_stats().await?;
    let directory = app
        .path()
        .app_data_dir()
        .map_err(|_| "app data directory unavailable".to_owned())?
        .join("media-cache");
    stats.media_bytes = directory_size(&directory).await;
    Ok(stats)
}

#[tauri::command]
async fn clear_media_cache(app: tauri::AppHandle) -> Result<(), String> {
    let directory = app
        .path()
        .app_data_dir()
        .map_err(|_| "app data directory unavailable".to_owned())?
        .join("media-cache");
    if tokio::fs::try_exists(&directory).await.unwrap_or(false) {
        tokio::fs::remove_dir_all(&directory)
            .await
            .map_err(|_| "failed to clear media cache".to_owned())?;
    }
    tokio::fs::create_dir_all(&directory)
        .await
        .map_err(|_| "failed to recreate media cache".to_owned())
}

#[tauri::command]
async fn local_cache_encryption_status(store: State<'_, LocalStore>) -> Result<bool, String> {
    Ok(store.encryption_enabled().await)
}

#[tauri::command]
async fn set_local_cache_encryption(
    store: State<'_, LocalStore>,
    enabled: bool,
) -> Result<bool, String> {
    store.set_encryption_enabled(enabled).await
}

async fn directory_size(root: &std::path::Path) -> u64 {
    let mut total = 0_u64;
    let mut pending = vec![root.to_owned()];
    while let Some(directory) = pending.pop() {
        let Ok(mut entries) = tokio::fs::read_dir(directory).await else {
            continue;
        };
        while let Ok(Some(entry)) = entries.next_entry().await {
            let Ok(metadata) = entry.metadata().await else {
                continue;
            };
            if metadata.is_file() {
                total = total.saturating_add(metadata.len());
            } else if metadata.is_dir() && !metadata.file_type().is_symlink() {
                pending.push(entry.path());
            }
        }
    }
    total
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_log::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_autostart::Builder::new().build())
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            let directory = app.path().app_data_dir()?;
            fs::create_dir_all(&directory)?;
            let store = tauri::async_runtime::block_on(LocalStore::open(
                &directory.join("iamrust.sqlite3"),
            ))
            .map_err(std::io::Error::other)?;
            app.manage(store);
            app.manage(remote_api::RemoteApi::new()?);
            desktop::setup(app)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event
                && desktop::should_close_to_tray(window.app_handle())
            {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            remote_api::remote_login,
            remote_api::remote_register,
            remote_api::remote_begin_qr_login,
            remote_api::remote_poll_qr_login,
            remote_api::remote_restore,
            remote_api::remote_logout,
            remote_api::remote_request_password_reset,
            remote_api::remote_confirm_password_reset,
            remote_api::remote_request,
            remote_api::remote_upload,
            remote_api::remote_download_attachment,
            remote_api::reveal_download,
            cache_bootstrap,
            load_cached_bootstrap,
            cache_messages,
            load_cached_messages,
            save_draft,
            enqueue_outbox,
            ready_outbox,
            acknowledge_outbox,
            clear_account_cache,
            cache_stats,
            clear_media_cache,
            local_cache_encryption_status,
            set_local_cache_encryption,
            desktop::set_tray_unread,
            desktop::set_close_to_tray
        ])
        .run(tauri::generate_context!())
        .expect("failed to run I Am Rust desktop application");
}
