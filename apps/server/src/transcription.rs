use std::time::Duration;

use anyhow::{Context, ensure};
use reqwest::multipart::{Form, Part};
use serde::Deserialize;
use url::Url;

#[derive(Debug, Clone)]
pub struct Transcriber {
    endpoint: Url,
    api_key: Option<String>,
    model: String,
    client: reqwest::Client,
}

#[derive(Debug, Deserialize)]
struct TranscriptionResult {
    text: String,
    #[serde(default)]
    language: Option<String>,
}

impl Transcriber {
    pub fn new(
        endpoint: &str,
        api_key: Option<String>,
        model: String,
        production: bool,
    ) -> anyhow::Result<Self> {
        let endpoint = Url::parse(endpoint).context("invalid transcription endpoint")?;
        ensure!(
            !production || endpoint.scheme() == "https",
            "transcription endpoint must use HTTPS in production"
        );
        ensure!(
            !model.trim().is_empty() && model.len() <= 100,
            "transcription model is invalid"
        );
        Ok(Self {
            endpoint,
            api_key,
            model,
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(90))
                .build()
                .context("failed to build transcription client")?,
        })
    }

    pub async fn transcribe(
        &self,
        bytes: Vec<u8>,
        file_name: &str,
        mime_type: &str,
    ) -> anyhow::Result<(String, Option<String>)> {
        let file = Part::bytes(bytes)
            .file_name(file_name.to_owned())
            .mime_str(mime_type)
            .context("invalid audio MIME type")?;
        let form = Form::new()
            .text("model", self.model.clone())
            .text("response_format", "verbose_json")
            .part("file", file);
        let mut request = self.client.post(self.endpoint.clone()).multipart(form);
        if let Some(api_key) = &self.api_key {
            request = request.bearer_auth(api_key);
        }
        let result = request
            .send()
            .await
            .context("transcription request failed")?
            .error_for_status()
            .context("transcription provider returned an error")?
            .json::<TranscriptionResult>()
            .await
            .context("invalid transcription response")?;
        ensure!(!result.text.trim().is_empty(), "transcription was empty");
        Ok((result.text, result.language))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn production_requires_https() {
        assert!(
            Transcriber::new(
                "http://speech.example.test/v1/audio/transcriptions",
                None,
                "whisper-1".to_owned(),
                true,
            )
            .is_err()
        );
    }
}
