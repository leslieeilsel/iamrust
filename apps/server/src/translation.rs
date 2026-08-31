use std::time::Duration;

use anyhow::{Context, ensure};
use serde::{Deserialize, Serialize};
use url::Url;

#[derive(Debug, Clone)]
pub struct Translator {
    endpoint: Url,
    api_key: Option<String>,
    client: reqwest::Client,
}

#[derive(Debug, Serialize)]
struct TranslationRequest<'a> {
    q: &'a str,
    source: &'static str,
    target: &'a str,
    format: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    api_key: Option<&'a str>,
}

#[derive(Debug, Deserialize)]
struct TranslationResult {
    #[serde(rename = "translatedText")]
    translated_text: String,
    #[serde(rename = "detectedLanguage", default)]
    detected_language: Option<DetectedLanguage>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum DetectedLanguage {
    Code(String),
    Details { language: String },
}

impl Translator {
    pub fn new(endpoint: &str, api_key: Option<String>, production: bool) -> anyhow::Result<Self> {
        let endpoint = Url::parse(endpoint).context("invalid translation endpoint")?;
        ensure!(
            !production || endpoint.scheme() == "https",
            "translation endpoint must use HTTPS in production"
        );
        Ok(Self {
            endpoint,
            api_key,
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(15))
                .build()
                .context("failed to build translation client")?,
        })
    }

    pub async fn translate(
        &self,
        text: &str,
        target_language: &str,
    ) -> anyhow::Result<(String, Option<String>)> {
        let response = self
            .client
            .post(self.endpoint.clone())
            .json(&TranslationRequest {
                q: text,
                source: "auto",
                target: target_language,
                format: "text",
                api_key: self.api_key.as_deref(),
            })
            .send()
            .await
            .context("translation request failed")?
            .error_for_status()
            .context("translation provider returned an error")?
            .json::<TranslationResult>()
            .await
            .context("invalid translation response")?;
        let source_language = response.detected_language.map(|language| match language {
            DetectedLanguage::Code(code) => code,
            DetectedLanguage::Details { language } => language,
        });
        Ok((response.translated_text, source_language))
    }
}
