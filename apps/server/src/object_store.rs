use std::fmt::{self, Write};

use anyhow::{Context, ensure};
use chrono::{DateTime, Utc};
use hmac::{Hmac, KeyInit as _, Mac};
use sha2::{Digest, Sha256};
use url::Url;

type HmacSha256 = Hmac<Sha256>;

#[derive(Clone)]
pub struct ObjectStore {
    endpoint: Url,
    bucket: String,
    region: String,
    access_key: String,
    secret_key: String,
    client: reqwest::Client,
}

impl fmt::Debug for ObjectStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ObjectStore")
            .field("endpoint", &self.endpoint)
            .field("bucket", &self.bucket)
            .field("region", &self.region)
            .field("access_key", &"[REDACTED]")
            .field("secret_key", &"[REDACTED]")
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresignedRequest {
    pub url: String,
    pub required_headers: Vec<(String, String)>,
}

impl ObjectStore {
    pub fn new(
        endpoint: &str,
        bucket: String,
        region: String,
        access_key: String,
        secret_key: String,
    ) -> anyhow::Result<Self> {
        let endpoint = Url::parse(endpoint).context("S3 endpoint must be an absolute URL")?;
        ensure!(
            matches!(endpoint.scheme(), "http" | "https"),
            "S3 endpoint must use HTTP or HTTPS"
        );
        ensure!(!bucket.is_empty(), "S3 bucket must not be empty");
        ensure!(!region.is_empty(), "S3 region must not be empty");
        ensure!(!access_key.is_empty(), "S3 access key must not be empty");
        ensure!(!secret_key.is_empty(), "S3 secret key must not be empty");
        Ok(Self {
            endpoint,
            bucket,
            region,
            access_key,
            secret_key,
            client: reqwest::Client::new(),
        })
    }

    pub fn presign_put(
        &self,
        key: &str,
        mime_type: &str,
        sha256: Option<&str>,
        now: DateTime<Utc>,
    ) -> anyhow::Result<PresignedRequest> {
        let mut headers = vec![("content-type".to_owned(), mime_type.to_owned())];
        if let Some(sha256) = sha256 {
            headers.push(("x-amz-meta-sha256".to_owned(), sha256.to_owned()));
        }
        self.presign("PUT", key, 600, now, headers)
    }

    pub fn presign_get(&self, key: &str, now: DateTime<Utc>) -> anyhow::Result<PresignedRequest> {
        self.presign("GET", key, 600, now, Vec::new())
    }

    pub async fn verify_object(
        &self,
        key: &str,
        expected_mime: &str,
        expected_size: u64,
        expected_sha256: Option<&str>,
        now: DateTime<Utc>,
    ) -> anyhow::Result<()> {
        let head = self.presign("HEAD", key, 60, now, Vec::new())?;
        let response = self
            .client
            .head(&head.url)
            .send()
            .await
            .context("object-store HEAD request failed")?;
        ensure!(
            response.status().is_success(),
            "uploaded object was not found"
        );
        let headers = response.headers();
        let content_length = headers
            .get(reqwest::header::CONTENT_LENGTH)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok())
            .context("object-store response omitted content length")?;
        ensure!(
            content_length == expected_size,
            "uploaded object size changed"
        );
        let content_type = headers
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(|value| value.split(';').next().unwrap_or(value).trim())
            .context("object-store response omitted content type")?;
        ensure!(
            content_type.eq_ignore_ascii_case(expected_mime),
            "uploaded object content type changed"
        );
        if let Some(expected_sha256) = expected_sha256 {
            let actual = headers
                .get("x-amz-meta-sha256")
                .and_then(|value| value.to_str().ok())
                .context("uploaded object omitted SHA-256 metadata")?;
            ensure!(
                actual.eq_ignore_ascii_case(expected_sha256),
                "uploaded object SHA-256 metadata changed"
            );
        }

        let get = self.presign_get(key, now)?;
        let response = self
            .client
            .get(&get.url)
            .header(reqwest::header::RANGE, "bytes=0-511")
            .send()
            .await
            .context("object-store range request failed")?;
        ensure!(
            response.status().is_success(),
            "uploaded object could not be inspected"
        );
        let prefix = response
            .bytes()
            .await
            .context("uploaded object prefix could not be read")?;
        validate_magic(expected_mime, &prefix)
    }

    pub async fn delete_object(&self, key: &str, now: DateTime<Utc>) -> anyhow::Result<()> {
        let request = self.presign("DELETE", key, 60, now, Vec::new())?;
        let response = self
            .client
            .delete(&request.url)
            .send()
            .await
            .context("object-store delete request failed")?;
        ensure!(
            response.status().is_success() || response.status() == reqwest::StatusCode::NOT_FOUND,
            "object-store delete request was rejected"
        );
        Ok(())
    }

    pub async fn read_object(
        &self,
        key: &str,
        maximum_bytes: u64,
        now: DateTime<Utc>,
    ) -> anyhow::Result<Vec<u8>> {
        let request = self.presign_get(key, now)?;
        let response = self
            .client
            .get(&request.url)
            .send()
            .await
            .context("object-store GET request failed")?
            .error_for_status()
            .context("object-store rejected GET request")?;
        if let Some(length) = response.content_length() {
            ensure!(length <= maximum_bytes, "object exceeds read limit");
        }
        let bytes = response
            .bytes()
            .await
            .context("object-store response could not be read")?;
        ensure!(
            u64::try_from(bytes.len()).unwrap_or(u64::MAX) <= maximum_bytes,
            "object exceeds read limit"
        );
        Ok(bytes.to_vec())
    }

    fn presign(
        &self,
        method: &str,
        key: &str,
        expires_seconds: u32,
        now: DateTime<Utc>,
        mut required_headers: Vec<(String, String)>,
    ) -> anyhow::Result<PresignedRequest> {
        ensure!(!key.is_empty() && !key.contains(".."), "unsafe object key");
        ensure!(
            (1..=604_800).contains(&expires_seconds),
            "presigned URL expiry is out of range"
        );
        required_headers.sort_by(|left, right| left.0.cmp(&right.0));

        let host = match self.endpoint.port() {
            Some(port) => format!(
                "{}:{port}",
                self.endpoint
                    .host_str()
                    .context("S3 endpoint needs a host")?
            ),
            None => self
                .endpoint
                .host_str()
                .context("S3 endpoint needs a host")?
                .to_owned(),
        };
        let base_path = self.endpoint.path().trim_end_matches('/');
        let canonical_uri = aws_encode_path(&format!("{base_path}/{}/{key}", self.bucket));
        let amz_date = now.format("%Y%m%dT%H%M%SZ").to_string();
        let date = now.format("%Y%m%d").to_string();
        let scope = format!("{date}/{}/s3/aws4_request", self.region);

        let mut signed_header_names = required_headers
            .iter()
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>();
        signed_header_names.push("host");
        signed_header_names.sort_unstable();
        let signed_headers = signed_header_names.join(";");

        let mut query = [
            ("X-Amz-Algorithm", "AWS4-HMAC-SHA256".to_owned()),
            ("X-Amz-Credential", format!("{}/{}", self.access_key, scope)),
            ("X-Amz-Date", amz_date.clone()),
            ("X-Amz-Expires", expires_seconds.to_string()),
            ("X-Amz-SignedHeaders", signed_headers.clone()),
        ];
        query.sort_by(|left, right| left.0.cmp(right.0));
        let canonical_query = query
            .iter()
            .map(|(name, value)| format!("{}={}", aws_encode(name), aws_encode(value)))
            .collect::<Vec<_>>()
            .join("&");

        let mut canonical_headers = required_headers
            .iter()
            .map(|(name, value)| format!("{}:{}\n", name.to_ascii_lowercase(), value.trim()))
            .collect::<Vec<_>>();
        canonical_headers.push(format!("host:{host}\n"));
        canonical_headers.sort_unstable();
        let canonical_headers = canonical_headers.concat();
        let canonical_request = format!(
            "{method}\n{canonical_uri}\n{canonical_query}\n{canonical_headers}\n{signed_headers}\nUNSIGNED-PAYLOAD"
        );
        let request_hash = hex(&Sha256::digest(canonical_request.as_bytes()));
        let string_to_sign = format!("AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{request_hash}");
        let date_key = hmac(
            format!("AWS4{}", self.secret_key).as_bytes(),
            date.as_bytes(),
        )?;
        let region_key = hmac(&date_key, self.region.as_bytes())?;
        let service_key = hmac(&region_key, b"s3")?;
        let signing_key = hmac(&service_key, b"aws4_request")?;
        let signature = hex(&hmac(&signing_key, string_to_sign.as_bytes())?);

        let mut url = self.endpoint.clone();
        url.set_path(&canonical_uri);
        url.set_query(Some(&format!(
            "{canonical_query}&X-Amz-Signature={signature}"
        )));
        Ok(PresignedRequest {
            url: url.into(),
            required_headers,
        })
    }
}

fn hmac(key: &[u8], value: &[u8]) -> anyhow::Result<Vec<u8>> {
    let mut mac = HmacSha256::new_from_slice(key).context("invalid HMAC key")?;
    mac.update(value);
    Ok(mac.finalize().into_bytes().to_vec())
}

fn aws_encode(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            output.push(char::from(byte));
        } else {
            let _ = write!(output, "%{byte:02X}");
        }
    }
    output
}

fn aws_encode_path(value: &str) -> String {
    value
        .split('/')
        .map(aws_encode)
        .collect::<Vec<_>>()
        .join("/")
}

fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

fn validate_magic(mime_type: &str, bytes: &[u8]) -> anyhow::Result<()> {
    let matches = match mime_type.to_ascii_lowercase().as_str() {
        "image/png" => bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
        "image/jpeg" => bytes.starts_with(&[0xff, 0xd8, 0xff]),
        "image/gif" => bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a"),
        "image/webp" => {
            bytes.starts_with(b"RIFF") && bytes.get(8..12).is_some_and(|value| value == b"WEBP")
        }
        "audio/ogg" | "video/ogg" => bytes.starts_with(b"OggS"),
        "audio/wav" | "audio/x-wav" => {
            bytes.starts_with(b"RIFF") && bytes.get(8..12).is_some_and(|value| value == b"WAVE")
        }
        "audio/mpeg" => {
            bytes.starts_with(b"ID3")
                || bytes
                    .get(..2)
                    .is_some_and(|value| value[0] == 0xff && value[1] & 0xe0 == 0xe0)
        }
        "video/mp4" => bytes.get(4..8).is_some_and(|value| value == b"ftyp"),
        "video/webm" | "audio/webm" => bytes.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]),
        "application/pdf" => bytes.starts_with(b"%PDF-"),
        _ => !is_executable(bytes),
    };
    ensure!(
        matches,
        "uploaded object signature does not match its MIME type"
    );
    Ok(())
}

fn is_executable(bytes: &[u8]) -> bool {
    bytes.starts_with(b"MZ")
        || bytes.starts_with(b"\x7fELF")
        || bytes.starts_with(&[0xfe, 0xed, 0xfa, 0xce])
        || bytes.starts_with(&[0xfe, 0xed, 0xfa, 0xcf])
        || bytes.starts_with(&[0xcf, 0xfa, 0xed, 0xfe])
        || bytes.starts_with(&[0xce, 0xfa, 0xed, 0xfe])
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    #[test]
    fn presigned_url_is_deterministic_and_never_contains_secret() {
        let store = ObjectStore::new(
            "http://127.0.0.1:9000",
            "iamrust-media".to_owned(),
            "us-east-1".to_owned(),
            "access".to_owned(),
            "do-not-leak".to_owned(),
        )
        .unwrap();
        let now = Utc.with_ymd_and_hms(2026, 8, 30, 12, 0, 0).unwrap();
        let first = store
            .presign_put("uploads/user/file", "image/png", Some("abc"), now)
            .unwrap();
        let second = store
            .presign_put("uploads/user/file", "image/png", Some("abc"), now)
            .unwrap();
        assert_eq!(first, second);
        assert!(first.url.contains("X-Amz-Signature="));
        assert!(!first.url.contains("do-not-leak"));
        assert!(
            first
                .required_headers
                .iter()
                .any(|(name, _)| name == "content-type")
        );
    }
}
