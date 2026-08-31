use std::{fmt, time::Duration};

use anyhow::{Context, ensure};
use chrono::{DateTime, Utc};
use iamrust_application::PasswordResetDelivery;
use lettre::{
    AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor,
    message::{Mailbox, header::ContentType},
};

#[derive(Clone)]
pub struct Mailer {
    transport: AsyncSmtpTransport<Tokio1Executor>,
    from: Mailbox,
}

impl fmt::Debug for Mailer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Mailer")
            .field("transport", &"[REDACTED]")
            .field("from", &self.from)
            .finish()
    }
}

impl Mailer {
    pub fn new(smtp_url: &str, from: &str, production: bool) -> anyhow::Result<Self> {
        let secure = smtp_url.starts_with("smtps://")
            || smtp_url
                .split_once('?')
                .is_some_and(|(_, query)| query.split('&').any(|item| item == "tls=required"));
        ensure!(
            !production || secure,
            "production SMTP must use SMTPS or require STARTTLS"
        );
        let transport = AsyncSmtpTransport::<Tokio1Executor>::from_url(smtp_url)
            .context("invalid IAMRUST_SMTP_URL")?
            .timeout(Some(Duration::from_secs(15)))
            .build();
        let from = from.parse().context("invalid IAMRUST_EMAIL_FROM")?;
        Ok(Self { transport, from })
    }

    pub async fn send_password_reset(&self, delivery: PasswordResetDelivery) -> anyhow::Result<()> {
        let recipient: Mailbox = delivery
            .email
            .parse()
            .context("invalid reset email address")?;
        let body = format!(
            concat!(
                "你正在重置 I Am Rust 的登录密码。\n\n",
                "验证码：{}\n",
                "有效期至：{} UTC\n\n",
                "如果这不是你的操作，请忽略此邮件；不要向任何人透露验证码。\n"
            ),
            delivery.reset_token,
            delivery.expires_at.format("%Y-%m-%d %H:%M:%S")
        );
        let message = Message::builder()
            .from(self.from.clone())
            .to(recipient)
            .subject("I Am Rust 密码重置验证码")
            .header(ContentType::TEXT_PLAIN)
            .body(body)
            .context("failed to build reset email")?;
        self.transport
            .send(message)
            .await
            .context("SMTP rejected reset email")?;
        Ok(())
    }

    pub async fn send_new_device_login(
        &self,
        email: String,
        device_name: String,
        platform: String,
        app_version: String,
        occurred_at: DateTime<Utc>,
    ) -> anyhow::Result<()> {
        let recipient: Mailbox = email.parse().context("invalid account email address")?;
        let body = format!(
            concat!(
                "你的 I Am Rust 账号刚刚在一个新会话中登录。\n\n",
                "设备：{}\n平台：{}\n客户端版本：{}\n时间：{} UTC\n\n",
                "如果这不是你的操作，请立即重置密码并在设备管理中撤销该设备。\n"
            ),
            device_name,
            platform,
            app_version,
            occurred_at.format("%Y-%m-%d %H:%M:%S")
        );
        let message = Message::builder()
            .from(self.from.clone())
            .to(recipient)
            .subject("I Am Rust 新设备登录提醒")
            .header(ContentType::TEXT_PLAIN)
            .body(body)
            .context("failed to build login alert email")?;
        self.transport
            .send(message)
            .await
            .context("SMTP rejected login alert email")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::Mailer;

    #[tokio::test]
    async fn production_rejects_plaintext_smtp() {
        assert!(Mailer::new("smtp://127.0.0.1:1025", "noreply@example.com", true).is_err());
        assert!(
            Mailer::new(
                "smtp://smtp.example.com:587?tls=required",
                "I Am Rust <noreply@example.com>",
                true,
            )
            .is_ok()
        );
    }
}
