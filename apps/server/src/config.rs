use std::{env, net::SocketAddr, time::Duration};

use anyhow::{Context, ensure};
use opentelemetry::{global, trace::TracerProvider as _};
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::{Resource, trace::SdkTracerProvider};
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Clone)]
pub struct ServerConfig {
    pub bind_addr: SocketAddr,
    pub database_url: Option<String>,
    pub s3_endpoint: String,
    pub s3_bucket: String,
    pub s3_region: String,
    pub s3_access_key: String,
    pub s3_secret_key: String,
    pub smtp_url: String,
    pub email_from: String,
    pub translation_url: Option<String>,
    pub translation_api_key: Option<String>,
    pub transcription_url: Option<String>,
    pub transcription_api_key: Option<String>,
    pub transcription_model: String,
    pub clamav_addr: Option<SocketAddr>,
    pub admin_token: Option<String>,
    pub data_encryption_key: String,
    pub production: bool,
}

impl ServerConfig {
    pub fn from_env() -> anyhow::Result<Self> {
        let bind_addr = env::var("IAMRUST_BIND_ADDR")
            .unwrap_or_else(|_| "127.0.0.1:3780".to_owned())
            .parse()
            .context("IAMRUST_BIND_ADDR must be a socket address")?;
        if let Ok(secret) = env::var("IAMRUST_JWT_SECRET") {
            ensure!(
                secret.len() >= 32,
                "IAMRUST_JWT_SECRET must contain at least 32 bytes"
            );
        }
        let database_url = env::var("IAMRUST_DATABASE_URL").ok();
        let production = env::var("IAMRUST_ENV").is_ok_and(|value| value == "production");
        let s3_endpoint =
            env::var("IAMRUST_S3_ENDPOINT").unwrap_or_else(|_| "http://127.0.0.1:9000".to_owned());
        let s3_bucket =
            env::var("IAMRUST_S3_BUCKET").unwrap_or_else(|_| "iamrust-media".to_owned());
        let s3_region = env::var("IAMRUST_S3_REGION").unwrap_or_else(|_| "us-east-1".to_owned());
        let s3_access_key =
            env::var("IAMRUST_S3_ACCESS_KEY").unwrap_or_else(|_| "iamrust".to_owned());
        let s3_secret_key =
            env::var("IAMRUST_S3_SECRET_KEY").unwrap_or_else(|_| "iamrust-dev-secret".to_owned());
        let smtp_url =
            env::var("IAMRUST_SMTP_URL").unwrap_or_else(|_| "smtp://127.0.0.1:1025".to_owned());
        let email_from = env::var("IAMRUST_EMAIL_FROM")
            .unwrap_or_else(|_| "I Am Rust <noreply@iamrust.local>".to_owned());
        let translation_url = env::var("IAMRUST_TRANSLATION_URL").ok();
        let translation_api_key = env::var("IAMRUST_TRANSLATION_API_KEY").ok();
        let transcription_url = env::var("IAMRUST_TRANSCRIPTION_URL").ok();
        let transcription_api_key = env::var("IAMRUST_TRANSCRIPTION_API_KEY").ok();
        let transcription_model =
            env::var("IAMRUST_TRANSCRIPTION_MODEL").unwrap_or_else(|_| "whisper-1".to_owned());
        let clamav_addr = env::var("IAMRUST_CLAMAV_ADDR")
            .ok()
            .map(|value| {
                value
                    .parse()
                    .context("IAMRUST_CLAMAV_ADDR must be a socket address")
            })
            .transpose()?;
        let admin_token = env::var("IAMRUST_ADMIN_TOKEN").ok();
        let data_encryption_key = env::var("IAMRUST_DATA_ENCRYPTION_KEY")
            .unwrap_or_else(|_| "iamrust-development-only-encryption-key".to_owned());
        ensure!(
            admin_token.as_ref().is_none_or(|token| token.len() >= 32),
            "IAMRUST_ADMIN_TOKEN must contain at least 32 bytes"
        );
        ensure!(
            data_encryption_key.len() >= 32,
            "IAMRUST_DATA_ENCRYPTION_KEY must contain at least 32 bytes"
        );
        ensure!(
            !production || env::var("IAMRUST_DATA_ENCRYPTION_KEY").is_ok(),
            "IAMRUST_DATA_ENCRYPTION_KEY is required in production"
        );
        ensure!(
            !production || database_url.is_some(),
            "IAMRUST_DATABASE_URL is required in production"
        );
        ensure!(
            !production
                || (env::var("IAMRUST_S3_ACCESS_KEY").is_ok()
                    && env::var("IAMRUST_S3_SECRET_KEY").is_ok()),
            "explicit S3 credentials are required in production"
        );
        Ok(Self {
            bind_addr,
            database_url,
            s3_endpoint,
            s3_bucket,
            s3_region,
            s3_access_key,
            s3_secret_key,
            smtp_url,
            email_from,
            translation_url,
            translation_api_key,
            transcription_url,
            transcription_api_key,
            transcription_model,
            clamav_addr,
            admin_token,
            data_encryption_key,
            production,
        })
    }

    pub fn object_store(&self) -> anyhow::Result<crate::object_store::ObjectStore> {
        crate::object_store::ObjectStore::new(
            &self.s3_endpoint,
            self.s3_bucket.clone(),
            self.s3_region.clone(),
            self.s3_access_key.clone(),
            self.s3_secret_key.clone(),
        )
        .context("invalid S3 configuration")
    }

    pub fn mailer(&self) -> anyhow::Result<crate::mailer::Mailer> {
        crate::mailer::Mailer::new(&self.smtp_url, &self.email_from, self.production)
            .context("invalid SMTP configuration")
    }

    pub fn translator(&self) -> anyhow::Result<Option<crate::translation::Translator>> {
        self.translation_url
            .as_deref()
            .map(|endpoint| {
                crate::translation::Translator::new(
                    endpoint,
                    self.translation_api_key.clone(),
                    self.production,
                )
            })
            .transpose()
    }

    pub fn transcriber(&self) -> anyhow::Result<Option<crate::transcription::Transcriber>> {
        self.transcription_url
            .as_deref()
            .map(|endpoint| {
                crate::transcription::Transcriber::new(
                    endpoint,
                    self.transcription_api_key.clone(),
                    self.transcription_model.clone(),
                    self.production,
                )
            })
            .transpose()
    }

    pub fn malware_scanner(&self) -> Option<crate::malware::MalwareScanner> {
        self.clamav_addr.map(crate::malware::MalwareScanner::new)
    }
}

pub fn init_tracing() -> anyhow::Result<Option<SdkTracerProvider>> {
    let filter = EnvFilter::try_from_env("IAMRUST_LOG")
        .unwrap_or_else(|_| EnvFilter::new("iamrust_server=info,tower_http=info"));
    let formatter = tracing_subscriber::fmt::layer()
        .json()
        .flatten_event(true)
        .with_target(false);
    let Ok(endpoint) =
        env::var("IAMRUST_OTLP_ENDPOINT").or_else(|_| env::var("OTEL_EXPORTER_OTLP_ENDPOINT"))
    else {
        tracing_subscriber::registry()
            .with(filter)
            .with(formatter)
            .try_init()
            .context("failed to initialize tracing subscriber")?;
        return Ok(None);
    };

    let exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_http()
        .with_endpoint(endpoint)
        .with_timeout(Duration::from_secs(5))
        .build()
        .context("failed to initialize OTLP span exporter")?;
    let provider = SdkTracerProvider::builder()
        .with_resource(
            Resource::builder()
                .with_service_name("iamrust-server")
                .build(),
        )
        .with_batch_exporter(exporter)
        .build();
    let tracer = provider.tracer("iamrust-server");
    global::set_tracer_provider(provider.clone());
    global::set_text_map_propagator(opentelemetry_sdk::propagation::TraceContextPropagator::new());
    tracing_subscriber::registry()
        .with(filter)
        .with(formatter)
        .with(tracing_opentelemetry::layer().with_tracer(tracer))
        .try_init()
        .context("failed to initialize tracing subscriber")?;
    Ok(Some(provider))
}
