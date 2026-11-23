//! Apache Iggy adapter over the official HTTP transport.
//!
//! Iggy is used as the durable cross-instance domain-event log when
//! `TSP_IGGY_URL` is configured. Redis remains responsible for short-lived
//! coordination state (leader lease and gateway sessions); Iggy owns the
//! replayable event stream.
//!
//! The adapter speaks the Iggy REST API (default port 3000), so it does not
//! force the native Iggy SDK/TLS stack into every deployment. Each API node
//! uses a unique standalone consumer on one partition, so every node receives
//! every event and filters its own `origin` envelope.
//!
//! API shape follows `core/server/server.http`: login, ensure stream/topic,
//! POST messages with a base64 payload, poll by consumer offset, commit offset.

use std::time::Duration;

use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use serde_json::{json, Value};
use tokio::sync::mpsc;

use crate::error::{AppError, AppResult};

#[derive(Clone)]
pub struct IggyBus {
    outgoing: mpsc::UnboundedSender<String>,
}

struct IggyHttp {
    client: reqwest::Client,
    base: String,
    token: String,
    stream_id: u32,
    topic_id: u32,
}

impl IggyBus {
    pub async fn connect(
        url: &str,
        node_id: &str,
        stream: &str,
        topic: &str,
        username: &str,
        password: &str,
    ) -> AppResult<(Self, mpsc::UnboundedReceiver<String>)> {
        let base = url.trim_end_matches('/').to_string();
        if !base.starts_with("http://") && !base.starts_with("https://") {
            return Err(AppError::internal(
                "TSP_IGGY_URL 必须是 Iggy HTTP 地址，例如 http://iggy:3000",
            ));
        }
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|error| {
                tracing::error!(%error, "failed to build Iggy HTTP client");
                AppError::internal("Iggy HTTP 客户端初始化失败")
            })?;

        let token = login(&client, &base, username, password).await?;
        let stream_id = ensure_stream(&client, &base, &token, stream).await?;
        let topic_id = ensure_topic(&client, &base, &token, stream_id, topic).await?;
        tracing::info!(
            node = %node_id,
            stream,
            topic,
            stream_id,
            topic_id,
            "Iggy event bus ready (HTTP)"
        );

        let http = IggyHttp {
            client: client.clone(),
            base: base.clone(),
            token: token.clone(),
            stream_id,
            topic_id,
        };

        let (outgoing, mut outgoing_rx) = mpsc::unbounded_channel::<String>();
        let producer_http = IggyHttp {
            client: client.clone(),
            base: base.clone(),
            token: token.clone(),
            stream_id,
            topic_id,
        };
        tokio::spawn(async move {
            while let Some(payload) = outgoing_rx.recv().await {
                if let Err(error) = producer_http.send(&payload).await {
                    tracing::warn!(%error, "failed to publish Iggy event");
                }
            }
        });

        let (incoming, incoming_rx) = mpsc::unbounded_channel::<String>();
        let consumer_id = consumer_id(node_id);
        tokio::spawn(async move {
            let mut offset: u64 = 0;
            loop {
                match http.poll(consumer_id, offset).await {
                    Ok((messages, next_offset)) => {
                        for payload in messages {
                            if incoming.send(payload).is_err() {
                                return;
                            }
                        }
                        if next_offset > offset {
                            if let Err(error) = http.commit(consumer_id, next_offset).await {
                                tracing::debug!(%error, "failed to commit Iggy offset");
                            }
                            offset = next_offset;
                        }
                        if offset == 0 {
                            tokio::time::sleep(Duration::from_millis(500)).await;
                        }
                    }
                    Err(error) => {
                        tracing::warn!(%error, "Iggy poll failed");
                        tokio::time::sleep(Duration::from_secs(2)).await;
                    }
                }
            }
        });

        Ok((Self { outgoing }, incoming_rx))
    }

    pub fn publish(&self, payload: String) {
        let _ = self.outgoing.send(payload);
    }
}

impl IggyHttp {
    async fn send(&self, payload: &str) -> AppResult<()> {
        let body = json!({
            "partitioning": {
                "kind": "partition_id",
                "value": BASE64.encode(0u32.to_be_bytes())
            },
            "messages": [{
                "id": 0,
                "payload": BASE64.encode(payload.as_bytes())
            }]
        });
        let response = self
            .client
            .post(self.url(&format!(
                "/streams/{}/topics/{}/messages",
                self.stream_id, self.topic_id
            )))
            .bearer_auth(&self.token)
            .json(&body)
            .send()
            .await
            .map_err(|error| {
                tracing::warn!(%error, "Iggy send request failed");
                AppError::internal("Iggy 发送失败")
            })?;
        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(AppError::internal(format!(
                "Iggy 发送失败（{status}）：{}",
                truncate(&text)
            )));
        }
        Ok(())
    }

    async fn poll(&self, consumer_id: u32, offset: u64) -> AppResult<(Vec<String>, u64)> {
        let url = self.url(&format!(
            "/streams/{}/topics/{}/messages?consumer_id={consumer_id}&partition_id=0&kind=offset&value={offset}&count=100&auto_commit=false",
            self.stream_id, self.topic_id
        ));
        let response = self
            .client
            .get(url)
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(|error| {
                tracing::warn!(%error, "Iggy poll request failed");
                AppError::internal("Iggy 拉取失败")
            })?;
        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(AppError::internal(format!(
                "Iggy 拉取失败（{status}）：{}",
                truncate(&text)
            )));
        }
        let value: Value = response.json().await.map_err(|error| {
            tracing::warn!(%error, "Iggy poll response is not JSON");
            AppError::internal("Iggy 响应解析失败")
        })?;
        let messages = value
            .get("messages")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut payloads = Vec::with_capacity(messages.len());
        let mut next_offset = offset;
        for message in messages {
            let encoded = message
                .get("payload")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if let Ok(bytes) = BASE64.decode(encoded) {
                if let Ok(text) = String::from_utf8(bytes) {
                    payloads.push(text);
                }
            }
            if let Some(message_offset) = message
                .get("header")
                .and_then(|header| header.get("offset"))
                .and_then(Value::as_u64)
            {
                next_offset = next_offset.max(message_offset + 1);
            } else {
                next_offset = next_offset.saturating_add(1);
            }
        }
        Ok((payloads, next_offset))
    }

    async fn commit(&self, consumer_id: u32, offset: u64) -> AppResult<()> {
        let response = self
            .client
            .put(self.url(&format!(
                "/streams/{}/topics/{}/consumer-offsets",
                self.stream_id, self.topic_id
            )))
            .bearer_auth(&self.token)
            .json(&json!({
                "consumer_id": consumer_id,
                "partition_id": 0,
                "offset": offset
            }))
            .send()
            .await
            .map_err(|error| {
                tracing::debug!(%error, "Iggy offset commit request failed");
                AppError::internal("Iggy offset 提交失败")
            })?;
        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(AppError::internal(format!(
                "Iggy offset 提交失败（{status}）：{}",
                truncate(&text)
            )));
        }
        Ok(())
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base, path)
    }
}

async fn login(
    client: &reqwest::Client,
    base: &str,
    username: &str,
    password: &str,
) -> AppResult<String> {
    let response = client
        .post(format!("{base}/users/login"))
        .json(&json!({ "username": username, "password": password }))
        .send()
        .await
        .map_err(|error| {
            tracing::error!(%error, "Iggy login request failed");
            AppError::internal("无法连接 Iggy")
        })?;
    if !response.status().is_success() {
        let status = response.status();
        return Err(AppError::internal(format!("Iggy 登录失败（{status}）")));
    }
    let value: Value = response.json().await.map_err(|error| {
        tracing::error!(%error, "Iggy login response is not JSON");
        AppError::internal("Iggy 登录响应解析失败")
    })?;
    value
        .get("access_token")
        .or_else(|| value.get("token"))
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .ok_or_else(|| AppError::internal("Iggy 登录响应缺少 access_token"))
}

async fn ensure_stream(
    client: &reqwest::Client,
    base: &str,
    token: &str,
    name: &str,
) -> AppResult<u32> {
    if let Some(id) = find_resource(client, base, token, "/streams", name, "stream_id").await? {
        return Ok(id);
    }
    let response = client
        .post(format!("{base}/streams"))
        .bearer_auth(token)
        .json(&json!({ "name": name }))
        .send()
        .await
        .map_err(|error| {
            tracing::error!(%error, "Iggy create stream failed");
            AppError::internal("Iggy Stream 创建失败")
        })?;
    if !response.status().is_success() {
        // A concurrent instance may have created it between GET and POST.
        if let Some(id) = find_resource(client, base, token, "/streams", name, "stream_id").await? {
            return Ok(id);
        }
        return Err(AppError::internal(format!(
            "Iggy Stream 创建失败（{}）",
            response.status()
        )));
    }
    parse_id(response.json::<Value>().await.ok(), "stream_id", "id")
        .ok_or_else(|| AppError::internal("Iggy Stream 响应缺少 id"))
}

async fn ensure_topic(
    client: &reqwest::Client,
    base: &str,
    token: &str,
    stream_id: u32,
    name: &str,
) -> AppResult<u32> {
    let path = format!("/streams/{stream_id}/topics");
    if let Some(id) = find_resource(client, base, token, &path, name, "topic_id").await? {
        return Ok(id);
    }
    let response = client
        .post(format!("{base}{path}"))
        .bearer_auth(token)
        .json(&json!({
            "name": name,
            "partitions_count": 1,
            "options": {
                "compression_algorithm": "none",
                "message_expiry": "30 days",
                "max_topic_size": "10 GiB"
            }
        }))
        .send()
        .await
        .map_err(|error| {
            tracing::error!(%error, "Iggy create topic failed");
            AppError::internal("Iggy Topic 创建失败")
        })?;
    if !response.status().is_success() {
        if let Some(id) = find_resource(client, base, token, &path, name, "topic_id").await? {
            return Ok(id);
        }
        return Err(AppError::internal(format!(
            "Iggy Topic 创建失败（{}）",
            response.status()
        )));
    }
    parse_id(response.json::<Value>().await.ok(), "topic_id", "id")
        .ok_or_else(|| AppError::internal("Iggy Topic 响应缺少 id"))
}

async fn find_resource(
    client: &reqwest::Client,
    base: &str,
    token: &str,
    path: &str,
    name: &str,
    id_key: &str,
) -> AppResult<Option<u32>> {
    let response = client
        .get(format!("{base}{path}"))
        .bearer_auth(token)
        .send()
        .await
        .map_err(|error| {
            tracing::debug!(%error, "Iggy list request failed");
            AppError::internal("Iggy 资源查询失败")
        })?;
    if !response.status().is_success() {
        return Ok(None);
    }
    let value: Value = response.json().await.unwrap_or(Value::Null);
    let items = if let Some(items) = value.as_array() {
        items.clone()
    } else {
        value
            .get("items")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    };
    for item in items {
        if item.get("name").and_then(Value::as_str) == Some(name) {
            return Ok(parse_id(Some(item), id_key, "id"));
        }
    }
    Ok(None)
}

fn parse_id(value: Option<Value>, key: &str, fallback: &str) -> Option<u32> {
    let value = value?;
    value
        .get(key)
        .or_else(|| value.get(fallback))
        .and_then(Value::as_u64)
        .and_then(|id| u32::try_from(id).ok())
}

fn consumer_id(node_id: &str) -> u32 {
    // FNV-1a; stable across restarts and distinct per node.
    let mut hash: u32 = 2_166_136_261;
    for byte in node_id.as_bytes() {
        hash ^= u32::from(*byte);
        hash = hash.wrapping_mul(16_777_619);
    }
    // Iggy consumer IDs are 32-bit; avoid zero which some deployments reserve.
    hash.max(1)
}

fn truncate(value: &str) -> String {
    value.chars().take(240).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn consumer_ids_are_stable_and_distinct() {
        assert_eq!(consumer_id("api-0"), consumer_id("api-0"));
        assert_ne!(consumer_id("api-0"), consumer_id("api-1"));
        assert_ne!(consumer_id(""), 0);
    }

    #[test]
    fn iggy_partition_payload_is_zero() {
        let encoded = BASE64.encode(0u32.to_be_bytes());
        assert_eq!(encoded, "AAAAAA==");
        assert_eq!(BASE64.decode(encoded).unwrap(), 0u32.to_be_bytes());
    }
}
