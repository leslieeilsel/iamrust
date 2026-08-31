use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use tauri::{
    App, AppHandle, Emitter, Manager,
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};

pub struct DesktopState {
    unread_item: MenuItem<tauri::Wry>,
    mute_item: CheckMenuItem<tauri::Wry>,
    notification_muted: AtomicBool,
    close_to_tray: AtomicBool,
}

impl DesktopState {
    fn new(unread_item: MenuItem<tauri::Wry>, mute_item: CheckMenuItem<tauri::Wry>) -> Self {
        Self {
            unread_item,
            mute_item,
            notification_muted: AtomicBool::new(false),
            close_to_tray: AtomicBool::new(true),
        }
    }
}

pub fn setup(app: &mut App) -> tauri::Result<()> {
    let open_item = MenuItem::with_id(app, "open", "打开 I Am Rust", true, None::<&str>)?;
    let unread_item = MenuItem::with_id(app, "unread", "未读消息：0", false, None::<&str>)?;
    let mute_item = CheckMenuItem::with_id(app, "mute", "通知静音", true, false, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let quit_item = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[&open_item, &unread_item, &mute_item, &separator, &quit_item],
    )?;

    app.manage(Arc::new(DesktopState::new(
        unread_item.clone(),
        mute_item.clone(),
    )));

    let mut tray = TrayIconBuilder::new()
        .menu(&menu)
        .show_menu_on_left_click(false)
        .tooltip("I Am Rust")
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" => show_main_window(app),
            "mute" => toggle_notification_mute(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                }
            ) {
                show_main_window(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon().cloned() {
        tray = tray.icon(icon);
    }
    tray.build(app)?;
    Ok(())
}

pub fn should_close_to_tray(app: &AppHandle) -> bool {
    app.state::<Arc<DesktopState>>()
        .close_to_tray
        .load(Ordering::Relaxed)
}

#[tauri::command]
pub fn set_tray_unread(app: AppHandle, count: u32) -> Result<(), String> {
    let state = app.state::<Arc<DesktopState>>();
    let label = if count > 999 {
        "未读消息：999+".to_owned()
    } else {
        format!("未读消息：{count}")
    };
    state
        .unread_item
        .set_text(label)
        .map_err(|_| "failed to update tray".to_owned())?;
    if let Some(tray) = app.tray_by_id("main") {
        let title = (count > 0).then(|| count.min(999).to_string());
        let _ = tray.set_title(title.as_deref());
        let _ = tray.set_tooltip(Some(if count > 0 {
            format!("I Am Rust · {count} 条未读")
        } else {
            "I Am Rust".to_owned()
        }));
    }
    Ok(())
}

#[tauri::command]
pub fn set_close_to_tray(app: AppHandle, enabled: bool) {
    app.state::<Arc<DesktopState>>()
        .close_to_tray
        .store(enabled, Ordering::Relaxed);
}

fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn toggle_notification_mute(app: &AppHandle) {
    let state = app.state::<Arc<DesktopState>>();
    let muted = !state.notification_muted.fetch_xor(true, Ordering::Relaxed);
    let _ = state.mute_item.set_checked(muted);
    let _ = app.emit("notification-muted", muted);
}
