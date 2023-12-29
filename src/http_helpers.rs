use anyhow::Context;
use std::time::Duration;
use thiserror::Error;

#[derive(Error, Debug)]
#[error("Hub is Locked for maintenance. Response: {body}")]
pub struct LockedError {
    pub body: String,
}

pub async fn json_body<T: serde::de::DeserializeOwned>(
    response: reqwest::Response,
) -> anyhow::Result<T> {
    let data = response.bytes().await.context("ready response body")?;
    serde_json::from_slice(&data).with_context(|| {
        format!(
            "parsing response as json: {}",
            String::from_utf8_lossy(&data)
        )
    })
}

pub async fn get_request_with_json_response<T: reqwest::IntoUrl, R: serde::de::DeserializeOwned>(
    url: T,
) -> anyhow::Result<R> {
    let response = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()?
        .request(reqwest::Method::GET, url)
        .send()
        .await?;

    let status = response.status();
    if !status.is_success() {
        let url = response.url().clone();
        let body_bytes = response.bytes().await.with_context(|| {
            format!(
                "request status {}: {}, and failed to read response body",
                status.as_u16(),
                status.canonical_reason().unwrap_or("")
            )
        })?;

        if status.as_u16() == 423 {
            let body = String::from_utf8_lossy(&body_bytes).to_string();
            return Err(LockedError { body }).with_context(move || format!("GET {url}"));
        }

        anyhow::bail!(
            "request status {}: {}. Response body: {}",
            status.as_u16(),
            status.canonical_reason().unwrap_or(""),
            String::from_utf8_lossy(&body_bytes)
        );
    }
    json_body(response).await.with_context(|| {
        format!(
            "request status {}: {}",
            status.as_u16(),
            status.canonical_reason().unwrap_or("")
        )
    })
}

pub async fn request_with_json_response<
    T: reqwest::IntoUrl,
    B: serde::Serialize,
    R: serde::de::DeserializeOwned,
>(
    method: reqwest::Method,
    url: T,
    body: &B,
) -> anyhow::Result<R> {
    let response = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()?
        .request(method, url)
        .json(body)
        .send()
        .await?;

    let status = response.status();
    if !status.is_success() {
        let body_bytes = response.bytes().await.with_context(|| {
            format!(
                "request status {}: {}, and failed to read response body",
                status.as_u16(),
                status.canonical_reason().unwrap_or("")
            )
        })?;
        anyhow::bail!(
            "request status {}: {}. Response body: {}",
            status.as_u16(),
            status.canonical_reason().unwrap_or(""),
            String::from_utf8_lossy(&body_bytes)
        );
    }
    json_body(response).await.with_context(|| {
        format!(
            "request status {}: {}",
            status.as_u16(),
            status.canonical_reason().unwrap_or("")
        )
    })
}
