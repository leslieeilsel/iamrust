use anyhow::{Context, bail};
use reqwest::{Client, Method, Response};
use serde_json::json;
use uuid::Uuid;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut arguments = std::env::args().skip(1);
    let command = arguments.next().unwrap_or_else(|| "help".to_owned());
    let base_url = std::env::var("IAMRUST_ADMIN_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:3780".to_owned())
        .trim_end_matches('/')
        .to_owned();
    let token = std::env::var("IAMRUST_ADMIN_TOKEN")
        .context("IAMRUST_ADMIN_TOKEN is required and is never accepted as a CLI argument")?;
    let client = Client::builder()
        .https_only(base_url.starts_with("https://"))
        .timeout(std::time::Duration::from_secs(15))
        .build()?;

    match command.as_str() {
        "suspend" | "restore" => {
            let user_id = required_user_id(arguments.next())?;
            let response = client
                .post(format!(
                    "{base_url}/api/v1/admin/users/{user_id}/suspension"
                ))
                .header("x-admin-token", &token)
                .json(&json!({ "suspended": command == "suspend" }))
                .send()
                .await?;
            ensure_success(response).await?;
            println!("user {user_id}: {command} completed");
        }
        "revoke-sessions" => {
            let user_id = required_user_id(arguments.next())?;
            let response = client
                .post(format!(
                    "{base_url}/api/v1/admin/users/{user_id}/sessions/revoke"
                ))
                .header("x-admin-token", &token)
                .send()
                .await?;
            ensure_success(response).await?;
            println!("user {user_id}: all sessions revoked");
        }
        "audit" => {
            let limit = arguments
                .next()
                .map(|value| {
                    value
                        .parse::<usize>()
                        .context("audit limit must be a number")
                })
                .transpose()?
                .unwrap_or(100)
                .clamp(1, 500);
            let response = client
                .request(
                    Method::GET,
                    format!("{base_url}/api/v1/admin/audit?limit={limit}"),
                )
                .header("x-admin-token", &token)
                .send()
                .await?;
            let response = ensure_success(response).await?;
            let payload: serde_json::Value = response.json().await?;
            println!("{}", serde_json::to_string_pretty(&payload)?);
        }
        _ => {
            println!(
                "Usage:\n  iamrust-admin suspend <user-uuid>\n  iamrust-admin restore <user-uuid>\n  iamrust-admin revoke-sessions <user-uuid>\n  iamrust-admin audit [limit]\n\nEnvironment: IAMRUST_ADMIN_URL, IAMRUST_ADMIN_TOKEN"
            );
        }
    }
    Ok(())
}

fn required_user_id(value: Option<String>) -> anyhow::Result<Uuid> {
    let value = value.context("a user UUID is required")?;
    Uuid::parse_str(&value).context("user ID must be a UUID")
}

async fn ensure_success(response: Response) -> anyhow::Result<Response> {
    if response.status().is_success() {
        return Ok(response);
    }
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    bail!("admin API returned {status}: {body}")
}
