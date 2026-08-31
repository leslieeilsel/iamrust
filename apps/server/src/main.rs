mod api;
mod config;
mod error;
mod mailer;
mod malware;
mod object_store;
mod openapi;
mod transcription;
mod translation;
mod websocket;

use anyhow::Context;
use iamrust_application::ChatService;
use sqlx::postgres::PgPoolOptions;
use tracing::info;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let tracing_provider = config::init_tracing()?;
    let config = config::ServerConfig::from_env()?;
    let object_store = config.object_store()?;
    let mailer = config.mailer()?;
    let translator = config.translator()?;
    let transcriber = config.transcriber()?;
    let malware_scanner = config.malware_scanner();
    let state = if let Some(database_url) = &config.database_url {
        let pool = PgPoolOptions::new()
            .max_connections(20)
            .acquire_timeout(std::time::Duration::from_secs(5))
            .connect(database_url)
            .await
            .context("failed to connect to PostgreSQL")?;
        sqlx::migrate!("../../migrations")
            .run(&pool)
            .await
            .context("failed to migrate PostgreSQL")?;
        let service = ChatService::postgres(pool.clone())
            .await
            .context("failed to restore application state")?
            .with_data_encryption_key(&config.data_encryption_key);
        info!("PostgreSQL migrations are current");
        api::AppState::new(service)
            .with_admin_token(config.admin_token.clone())
            .with_database(pool)
            .with_object_store(object_store.clone())
            .with_mailer(mailer.clone())
            .with_translator(translator.clone())
            .with_transcriber(transcriber.clone())
            .with_malware_scanner(malware_scanner.clone())
    } else {
        tracing::warn!("IAMRUST_DATABASE_URL is not set; using ephemeral development storage");
        api::AppState::new(ChatService::new().with_data_encryption_key(&config.data_encryption_key))
            .with_admin_token(config.admin_token.clone())
            .with_object_store(object_store.clone())
            .with_mailer(mailer)
            .with_translator(translator)
            .with_transcriber(transcriber)
            .with_malware_scanner(malware_scanner)
    };
    spawn_scheduled_message_worker(state.service.clone());
    spawn_attachment_cleanup_worker(state.service.clone(), object_store);
    let app = api::router(state);
    let listener = tokio::net::TcpListener::bind(config.bind_addr)
        .await
        .with_context(|| format!("failed to bind {}", config.bind_addr))?;
    info!(address = %config.bind_addr, "I Am Rust server listening");
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await
    .context("server stopped unexpectedly")?;
    if let Some(provider) = tracing_provider
        && let Err(error) = provider.shutdown()
    {
        tracing::warn!(%error, "failed to flush OpenTelemetry spans");
    }
    Ok(())
}

fn spawn_scheduled_message_worker(service: ChatService) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            let delivered = service.deliver_due_messages(chrono::Utc::now()).await;
            if delivered > 0 {
                tracing::info!(delivered, "delivered scheduled messages");
            }
        }
    });
}

fn spawn_attachment_cleanup_worker(service: ChatService, object_store: object_store::ObjectStore) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            let now = chrono::Utc::now();
            let Ok(keys) = service.cleanup_expired_attachments(now).await else {
                tracing::warn!("failed to clean expired attachment authorizations");
                continue;
            };
            for key in keys {
                if object_store.delete_object(&key, now).await.is_err() {
                    tracing::warn!(object_key = %key, "failed to delete expired object");
                }
            }
        }
    });
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
}
