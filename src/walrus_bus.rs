//! Optional Walrus distributed-log adapter.
//!
//! Walrus exposes a length-prefixed TCP protocol:
//! `REGISTER <topic>`, `PUT <topic> <payload>`, `GET <topic>`.
//!
//! This adapter publishes domain events to Walrus as a durable downstream log.
//! It deliberately does not use `GET` for WebSocket fan-out: Walrus topics use
//! a shared cursor, so multiple API nodes would compete for messages. Redis or
//! Iggy remains the realtime broadcast bus; Walrus is the durable archive /
//! downstream stream.

use std::time::Duration;

use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    sync::mpsc,
};

use crate::error::{AppError, AppResult};

#[derive(Clone)]
pub struct WalrusBus {
    sender: mpsc::UnboundedSender<String>,
}

impl WalrusBus {
    pub async fn connect(addr: &str, topic: &str) -> AppResult<Self> {
        let mut stream = TcpStream::connect(addr).await.map_err(|error| {
            tracing::error!(%error, %addr, "failed to connect Walrus");
            AppError::internal("无法连接 Walrus")
        })?;
        register(&mut stream, topic).await?;
        tracing::info!(%addr, %topic, "Walrus event archive ready");

        let (sender, mut receiver) = mpsc::unbounded_channel::<String>();
        let address = addr.to_string();
        let topic_name = topic.to_string();
        tokio::spawn(async move {
            let mut connection = Some(stream);
            while let Some(payload) = receiver.recv().await {
                if connection.is_none() {
                    match TcpStream::connect(&address).await {
                        Ok(mut stream) => {
                            if register(&mut stream, &topic_name).await.is_ok() {
                                connection = Some(stream);
                            }
                        }
                        Err(error) => {
                            tracing::warn!(%error, %address, "Walrus reconnect failed");
                        }
                    }
                }
                let Some(stream) = connection.as_mut() else {
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    continue;
                };
                if put(stream, &topic_name, &payload).await.is_err() {
                    connection = None;
                }
            }
        });
        Ok(Self { sender })
    }

    pub fn publish(&self, payload: String) {
        let _ = self.sender.send(payload);
    }
}

async fn register(stream: &mut TcpStream, topic: &str) -> AppResult<()> {
    let response = command(stream, &format!("REGISTER {topic}")).await?;
    if response.starts_with("OK") {
        Ok(())
    } else {
        Err(AppError::internal(format!(
            "Walrus REGISTER 失败：{}",
            truncate(&response)
        )))
    }
}

async fn put(stream: &mut TcpStream, topic: &str, payload: &str) -> AppResult<()> {
    let response = command(stream, &format!("PUT {topic} {payload}")).await?;
    if response.starts_with("OK") {
        Ok(())
    } else {
        Err(AppError::internal(format!(
            "Walrus PUT 失败：{}",
            truncate(&response)
        )))
    }
}

async fn command(stream: &mut TcpStream, command: &str) -> AppResult<String> {
    let length = u32::try_from(command.len())
        .map_err(|_| AppError::bad_request("Walrus 命令过长"))?
        .to_le_bytes();
    stream.write_all(&length).await.map_err(|error| {
        tracing::warn!(%error, "Walrus command write failed");
        AppError::internal("Walrus 写入失败")
    })?;
    stream
        .write_all(command.as_bytes())
        .await
        .map_err(|error| {
            tracing::warn!(%error, "Walrus command write failed");
            AppError::internal("Walrus 写入失败")
        })?;

    let mut length_bytes = [0u8; 4];
    tokio::time::timeout(Duration::from_secs(5), stream.read_exact(&mut length_bytes))
        .await
        .map_err(|_| AppError::internal("Walrus 响应超时"))?
        .map_err(|error| {
            tracing::warn!(%error, "Walrus response length read failed");
            AppError::internal("Walrus 响应读取失败")
        })?;
    let length = u32::from_le_bytes(length_bytes) as usize;
    if length > 8 * 1024 * 1024 {
        return Err(AppError::internal("Walrus 响应过大"));
    }
    let mut body = vec![0u8; length];
    tokio::time::timeout(Duration::from_secs(5), stream.read_exact(&mut body))
        .await
        .map_err(|_| AppError::internal("Walrus 响应超时"))?
        .map_err(|error| {
            tracing::warn!(%error, "Walrus response body read failed");
            AppError::internal("Walrus 响应读取失败")
        })?;
    Ok(String::from_utf8_lossy(&body).into_owned())
}

fn truncate(value: &str) -> String {
    value.chars().take(200).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncation_is_bounded() {
        assert_eq!(truncate(&"x".repeat(300)).len(), 200);
    }
}
