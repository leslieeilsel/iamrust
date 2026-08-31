use std::{
    fmt, fs,
    io::Cursor,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context as _, Result, anyhow};
use gpui::{App, Global, Timer};
use single_instance::SingleInstance;
use tray_icon::{
    Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent, TrayIconId,
    menu::{Menu, MenuEvent, MenuItem},
};

const ACTIVATION_REQUEST_DIRECTORY: &str = ".activation-request";
#[cfg(target_os = "macos")]
const INSTANCE_LOCK_FILE: &str = ".instance.lock";
#[cfg(not(target_os = "macos"))]
const INSTANCE_NAME: &str = "app.iamrust.desktop.instance";
const TRAY_ICON_ID: &str = "app.iamrust.desktop.tray";
const SHOW_MENU_ID: &str = "app.iamrust.desktop.tray.show";
const QUIT_MENU_ID: &str = "app.iamrust.desktop.tray.quit";
const TRAY_ICON_SIZE: u32 = 64;
const DESKTOP_EVENT_INTERVAL: Duration = Duration::from_millis(250);
const LOGO_PNG: &[u8] = include_bytes!("../../../assets/branding/iamrust-logo-monochrome.png");

pub enum InstanceLaunch {
    Primary(PrimaryInstance),
    Secondary,
}

impl fmt::Debug for InstanceLaunch {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Primary(instance) => formatter.debug_tuple("Primary").field(instance).finish(),
            Self::Secondary => formatter.write_str("Secondary"),
        }
    }
}

pub struct PrimaryInstance {
    instance: SingleInstance,
    activation_request: PathBuf,
}

impl fmt::Debug for PrimaryInstance {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PrimaryInstance")
            .field("activation_request", &self.activation_request)
            .finish_non_exhaustive()
    }
}

pub fn acquire_instance(data_directory: &Path) -> Result<InstanceLaunch> {
    fs::create_dir_all(data_directory).with_context(|| {
        format!(
            "failed to create application data directory {}",
            data_directory.display()
        )
    })?;
    let activation_request = data_directory.join(ACTIVATION_REQUEST_DIRECTORY);
    let lock_name = instance_lock_name(data_directory);
    let instance = SingleInstance::new(&lock_name)
        .with_context(|| format!("failed to acquire desktop instance lock {lock_name}"))?;
    if !instance.is_single() {
        request_activation(&activation_request)?;
        return Ok(InstanceLaunch::Secondary);
    }

    // A crashed process can leave this empty marker behind. Consuming it here
    // prevents an old request from stealing focus during the next launch.
    let _ = take_activation_request(&activation_request);
    Ok(InstanceLaunch::Primary(PrimaryInstance {
        instance,
        activation_request,
    }))
}

#[cfg(target_os = "macos")]
fn instance_lock_name(data_directory: &Path) -> String {
    data_directory
        .join(INSTANCE_LOCK_FILE)
        .to_string_lossy()
        .into_owned()
}

#[cfg(not(target_os = "macos"))]
fn instance_lock_name(_data_directory: &Path) -> String {
    INSTANCE_NAME.to_owned()
}

fn request_activation(path: &Path) -> Result<()> {
    match fs::create_dir(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists && path.is_dir() => Ok(()),
        Err(error) => Err(error)
            .with_context(|| format!("failed to request activation through {}", path.display())),
    }
}

fn take_activation_request(path: &Path) -> Result<bool> {
    match fs::remove_dir(path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error)
            .with_context(|| format!("failed to consume activation request {}", path.display())),
    }
}

struct DesktopIntegration {
    _instance: SingleInstance,
    _tray: Option<TrayIcon>,
    tray_id: Option<TrayIconId>,
    activation_request: PathBuf,
    activation_error_reported: bool,
}

impl Global for DesktopIntegration {}

impl DesktopIntegration {
    fn take_activation_request(&mut self) -> bool {
        match take_activation_request(&self.activation_request) {
            Ok(requested) => requested,
            Err(error) => {
                if !self.activation_error_reported {
                    eprintln!("I Am Rust activation warning: {error:#}");
                    self.activation_error_reported = true;
                }
                false
            }
        }
    }
}

#[derive(Debug)]
pub struct IntegrationStatus {
    pub tray_available: bool,
    pub warning: Option<String>,
}

pub fn install_integration(primary: PrimaryInstance, cx: &mut App) -> IntegrationStatus {
    let (tray, tray_error) = match build_tray_icon() {
        Ok(tray) => (Some(tray), None),
        Err(error) => (
            None,
            Some(format!("系统托盘初始化失败，应用仍可正常使用：{error:#}")),
        ),
    };
    let tray_id = tray.as_ref().map(|tray| tray.id().clone());
    let tray_available = tray_id.is_some();
    cx.set_global(DesktopIntegration {
        _instance: primary.instance,
        _tray: tray,
        tray_id,
        activation_request: primary.activation_request,
        activation_error_reported: false,
    });

    cx.spawn(async move |cx| {
        loop {
            Timer::after(DESKTOP_EVENT_INTERVAL).await;
            match cx.update(poll_desktop_events) {
                Ok(true) => {}
                Ok(false) | Err(_) => break,
            }
        }
    })
    .detach();
    IntegrationStatus {
        tray_available,
        warning: tray_error,
    }
}

fn build_tray_icon() -> Result<TrayIcon> {
    let icon = decode_tray_icon()?;
    let show = MenuItem::with_id(SHOW_MENU_ID, "显示 I Am Rust", true, None);
    let quit = MenuItem::with_id(QUIT_MENU_ID, "退出 I Am Rust", true, None);
    let menu = Menu::with_items(&[&show, &quit]).context("failed to build tray menu")?;
    TrayIconBuilder::new()
        .with_id(TRAY_ICON_ID)
        .with_tooltip("I Am Rust")
        .with_icon(icon)
        .with_icon_as_template(cfg!(target_os = "macos"))
        .with_menu_on_left_click(false)
        .with_menu(Box::new(menu))
        .build()
        .context("failed to create system tray icon")
}

fn decode_tray_icon() -> Result<Icon> {
    let mut decoder = png::Decoder::new(Cursor::new(LOGO_PNG));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder
        .read_info()
        .context("failed to read tray icon PNG")?;
    let buffer_size = reader
        .output_buffer_size()
        .ok_or_else(|| anyhow!("tray icon PNG exceeds the decoder limit"))?;
    let mut pixels = vec![0; buffer_size];
    let info = reader
        .next_frame(&mut pixels)
        .context("failed to decode tray icon PNG")?;
    let rgba = pixels_to_rgba(&pixels[..info.buffer_size()], info.color_type)?;
    let resized = resize_rgba_nearest(&rgba, info.width, info.height, TRAY_ICON_SIZE)?;
    Icon::from_rgba(resized, TRAY_ICON_SIZE, TRAY_ICON_SIZE)
        .context("failed to create native tray icon")
}

fn pixels_to_rgba(pixels: &[u8], color_type: png::ColorType) -> Result<Vec<u8>> {
    let components = match color_type {
        png::ColorType::Grayscale => 1,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        png::ColorType::Indexed => return Err(anyhow!("indexed tray icon was not expanded")),
    };
    if !pixels.len().is_multiple_of(components) {
        return Err(anyhow!("tray icon contains an incomplete pixel"));
    }
    let pixel_count = pixels.len() / components;
    let mut rgba = Vec::with_capacity(pixel_count.saturating_mul(4));
    match color_type {
        png::ColorType::Grayscale => {
            for gray in pixels {
                rgba.extend_from_slice(&[*gray, *gray, *gray, u8::MAX]);
            }
        }
        png::ColorType::GrayscaleAlpha => {
            for &[gray, alpha] in pixels.as_chunks::<2>().0 {
                rgba.extend_from_slice(&[gray, gray, gray, alpha]);
            }
        }
        png::ColorType::Rgb => {
            for &[red, green, blue] in pixels.as_chunks::<3>().0 {
                rgba.extend_from_slice(&[red, green, blue, u8::MAX]);
            }
        }
        png::ColorType::Rgba => rgba.extend_from_slice(pixels),
        png::ColorType::Indexed => unreachable!("indexed images are rejected above"),
    }
    Ok(rgba)
}

fn resize_rgba_nearest(source: &[u8], width: u32, height: u32, size: u32) -> Result<Vec<u8>> {
    if width == 0 || height == 0 || size == 0 {
        return Err(anyhow!("tray icon dimensions must be non-zero"));
    }
    let expected = usize::try_from(width)
        .ok()
        .and_then(|width| {
            usize::try_from(height)
                .ok()
                .map(|height| width * height * 4)
        })
        .ok_or_else(|| anyhow!("tray icon dimensions are too large"))?;
    if source.len() != expected {
        return Err(anyhow!("tray icon pixel buffer has an invalid length"));
    }

    let side = usize::try_from(size).context("tray icon size is unsupported")?;
    let source_width = usize::try_from(width).context("tray icon width is unsupported")?;
    let source_height = usize::try_from(height).context("tray icon height is unsupported")?;
    let mut resized = vec![0; side * side * 4];
    for y in 0..side {
        let source_y = y * source_height / side;
        for x in 0..side {
            let source_x = x * source_width / side;
            let source_offset = (source_y * source_width + source_x) * 4;
            let target_offset = (y * side + x) * 4;
            resized[target_offset..target_offset + 4]
                .copy_from_slice(&source[source_offset..source_offset + 4]);
        }
    }
    Ok(resized)
}

fn poll_desktop_events(cx: &mut App) -> bool {
    let (activation_requested, tray_id) = {
        let integration = cx.global_mut::<DesktopIntegration>();
        (
            integration.take_activation_request(),
            integration.tray_id.clone(),
        )
    };
    let mut show = activation_requested;
    let mut quit = false;

    while let Ok(event) = MenuEvent::receiver().try_recv() {
        if event.id == SHOW_MENU_ID {
            show = true;
        } else if event.id == QUIT_MENU_ID {
            quit = true;
        }
    }
    while let Ok(event) = TrayIconEvent::receiver().try_recv() {
        if tray_event_requests_activation(&event, tray_id.as_ref()) {
            show = true;
        }
    }

    if quit {
        cx.quit();
        return false;
    }
    if show {
        activate_main_window(cx);
    }
    true
}

fn tray_event_requests_activation(event: &TrayIconEvent, tray_id: Option<&TrayIconId>) -> bool {
    let Some(tray_id) = tray_id else {
        return false;
    };
    match event {
        TrayIconEvent::Click {
            id,
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            ..
        }
        | TrayIconEvent::DoubleClick {
            id,
            button: MouseButton::Left,
            ..
        } => id == tray_id,
        _ => false,
    }
}

pub fn activate_main_window(cx: &mut App) {
    cx.activate(true);
    if let Some(window_handle) = cx.windows().into_iter().next() {
        let _ = window_handle.update(cx, |_, window, _| window.activate_window());
    }
}

pub fn configure_close_to_tray(window: &gpui::Window, cx: &App) {
    window.on_window_should_close(cx, |window, cx| {
        window.minimize_window();
        #[cfg(target_os = "macos")]
        cx.hide();
        false
    });
}

pub fn show_message_notification(title: String, body: String, play_sound: bool) {
    let mut notification = notify_rust::Notification::new();
    notification
        .appname("I Am Rust")
        .summary(&title)
        .body(&body)
        .timeout(notify_rust::Timeout::Milliseconds(5_000));
    if play_sound {
        notification.sound_name(notification_sound_name());
    }
    let _ = notification.show();
}

#[cfg(target_os = "macos")]
const fn notification_sound_name() -> &'static str {
    "default"
}

#[cfg(target_os = "windows")]
const fn notification_sound_name() -> &'static str {
    "Default"
}

#[cfg(all(unix, not(target_os = "macos")))]
const fn notification_sound_name() -> &'static str {
    "message-new-instant"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activation_marker_is_idempotent_and_consumed_once() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let marker = directory.path().join(ACTIVATION_REQUEST_DIRECTORY);
        request_activation(&marker).expect("first activation request");
        request_activation(&marker).expect("duplicate activation request");
        assert!(take_activation_request(&marker).expect("consume marker"));
        assert!(!take_activation_request(&marker).expect("marker already consumed"));
    }

    #[test]
    fn bundled_logo_decodes_to_native_tray_size() {
        let mut decoder = png::Decoder::new(Cursor::new(LOGO_PNG));
        decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
        let mut reader = decoder.read_info().expect("logo header");
        let mut pixels = vec![0; reader.output_buffer_size().expect("logo buffer")];
        let info = reader.next_frame(&mut pixels).expect("logo pixels");
        let rgba = pixels_to_rgba(&pixels[..info.buffer_size()], info.color_type).expect("rgba");
        let resized = resize_rgba_nearest(&rgba, info.width, info.height, TRAY_ICON_SIZE)
            .expect("resized logo");
        assert_eq!(
            resized.len(),
            (TRAY_ICON_SIZE * TRAY_ICON_SIZE * 4) as usize
        );
        assert!(resized.as_chunks::<4>().0.iter().any(|pixel| pixel[3] > 0));
    }

    #[test]
    fn resize_rejects_mismatched_pixel_data() {
        let error = resize_rgba_nearest(&[0; 3], 1, 1, 64).expect_err("invalid pixels");
        assert!(error.to_string().contains("invalid length"));
    }
}
