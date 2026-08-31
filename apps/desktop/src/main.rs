mod api;
mod desktop;
mod model;
mod realtime;
mod shell;
mod ui;

use std::sync::Arc;

use gpui::{
    AppContext as _, Application, Bounds, TitlebarOptions, WindowBounds, WindowOptions, px, size,
};
use gpui_component::{Root, Theme, ThemeMode};
use iamrust_client_core::{LocalStore, default_data_directory};
use tokio::runtime::Runtime;

use crate::shell::{
    MIN_WINDOW_HEIGHT, MIN_WINDOW_WIDTH, WINDOW_PLACEMENT_SETTING, WindowPlacement,
};
use crate::ui::IamRustApp;

fn main() {
    let Some(primary_instance) = prepare_primary_instance() else {
        return;
    };
    run_application(primary_instance);
}

fn prepare_primary_instance() -> Option<desktop::PrimaryInstance> {
    let data_directory = match default_data_directory() {
        Ok(directory) => directory,
        Err(error) => {
            eprintln!("I Am Rust cannot resolve its data directory: {error}");
            return None;
        }
    };
    match desktop::acquire_instance(&data_directory) {
        Ok(desktop::InstanceLaunch::Primary(instance)) => Some(instance),
        Ok(desktop::InstanceLaunch::Secondary) => None,
        Err(error) => {
            eprintln!("I Am Rust cannot start desktop coordination: {error:#}");
            None
        }
    }
}

fn run_application(primary_instance: desktop::PrimaryInstance) {
    let runtime = Arc::new(Runtime::new().expect("failed to initialize client runtime"));
    let (store, cache_error) = match runtime.block_on(LocalStore::open_default()) {
        Ok(store) => (Some(store), None),
        Err(error) => (None, Some(error)),
    };
    let startup_theme = store
        .as_ref()
        .and_then(|store| {
            runtime
                .block_on(store.load_setting::<String>("ui.theme"))
                .ok()
                .flatten()
        })
        .filter(|mode| matches!(mode.as_str(), "system" | "light" | "dark"))
        .unwrap_or_else(|| "system".to_owned());
    let startup_placement = store.as_ref().and_then(|store| {
        runtime
            .block_on(store.load_setting::<WindowPlacement>(WINDOW_PLACEMENT_SETTING))
            .ok()
            .flatten()
    });

    let application = Application::new();
    application.on_reopen(desktop::activate_main_window);
    application.run(move |cx| {
        gpui_component::init(cx);
        let integration = desktop::install_integration(primary_instance, cx);
        if !integration.tray_available {
            cx.on_window_closed(|cx| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
        }
        let theme = match startup_theme.as_str() {
            "dark" => ThemeMode::Dark,
            "light" => ThemeMode::Light,
            _ => cx.window_appearance().into(),
        };
        Theme::change(theme, None, cx);

        let window_bounds = startup_placement
            .as_ref()
            .and_then(|placement| placement.restore(cx))
            .unwrap_or_else(|| {
                WindowBounds::Windowed(Bounds::centered(None, size(px(1180.), px(760.)), cx))
            });
        let runtime = runtime.clone();
        let store = store.clone();
        let cache_error = match (cache_error.clone(), integration.warning) {
            (Some(cache_error), Some(warning)) => Some(format!("{cache_error}\n{warning}")),
            (Some(cache_error), None) => Some(cache_error),
            (None, warning) => warning,
        };
        let theme_preference = startup_theme.clone();
        let close_to_tray = integration.tray_available;
        cx.spawn(async move |cx| {
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(window_bounds),
                    titlebar: Some(TitlebarOptions {
                        title: Some("I Am Rust".into()),
                        ..TitlebarOptions::default()
                    }),
                    app_id: Some("app.iamrust.desktop".to_owned()),
                    window_min_size: Some(size(px(MIN_WINDOW_WIDTH), px(MIN_WINDOW_HEIGHT))),
                    ..WindowOptions::default()
                },
                move |window, cx| {
                    if close_to_tray {
                        desktop::configure_close_to_tray(window, cx);
                    }
                    let app = cx.new(|cx| {
                        IamRustApp::new(
                            window,
                            cx,
                            runtime.clone(),
                            store.clone(),
                            cache_error.clone(),
                            theme_preference.clone(),
                        )
                    });
                    cx.new(|cx| Root::new(app, window, cx))
                },
            )?;
            Ok::<_, anyhow::Error>(())
        })
        .detach();
        cx.activate(true);
    });
}
