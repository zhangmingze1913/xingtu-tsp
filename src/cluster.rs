//! Cluster coordination primitives.
//!
//! The single-node profile keeps all state in-process. In cluster mode the
//! following state is shared through Redis:
//!
//! * gateway session ownership (`tsp:session:<phone>`)
//! * node liveness (`tsp:node:<node_id>`)
//! * singleton tasks (`tsp:leader:scheduler`, Redis lease)
//!
//! Business data lives in PostgreSQL, never in Redis. Redis is intentionally
//! treated as ephemeral coordination state.

use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

use futures_util::StreamExt;
use redis::{aio::ConnectionManager, AsyncCommands, Client, Script};
use serde::{Deserialize, Serialize};

use crate::{models::Terminal, raft::RaftControl};

type RaftBootstrap = (u64, String, Vec<(u64, String)>, bool);

use crate::error::{AppError, AppResult};

const LEADER_KEY: &str = "tsp:leader:scheduler";
const LEADER_TTL_MS: u64 = 15_000;
const NODE_TTL_SECONDS: u64 = 45;
const SESSION_TTL_SECONDS: u64 = 90;
const GATEWAY_CHANNEL_PREFIX: &str = "tsp:gateway:";
const EVENTS_CHANNEL: &str = "tsp:events";
const JT808_KICK_CHANNEL: &str = "tsp:jt808:kick";
const REVOKED_TOKEN_PREFIX: &str = "tsp:revoked:";
const TERMINAL_CACHE_PREFIX: &str = "tsp:terminal:";
const TERMINAL_CACHE_TTL_SECONDS: u64 = 60;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionInfo {
    pub node_id: String,
    pub peer: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct ClusterStatus {
    pub enabled: bool,
    pub node_id: String,
    pub leader: bool,
    pub redis_connected: bool,
    pub raft_enabled: bool,
    pub raft_node_id: Option<u64>,
    pub raft_leader: Option<u64>,
    pub raft_term: u64,
    pub raft_state: String,
    pub raft_last_log: Option<u64>,
}

#[derive(Clone)]
pub struct Cluster {
    inner: Option<Arc<ClusterInner>>,
    node_id: Arc<str>,
    leader: Arc<AtomicBool>,
    raft: Option<Arc<RaftControl>>,
}

struct ClusterInner {
    client: Client,
    publisher: ConnectionManager,
}

impl Cluster {
    /// Local/demo mode: no Redis, this node is always the singleton leader.
    pub fn disabled(node_id: impl Into<String>) -> Self {
        Self {
            inner: None,
            node_id: Arc::from(node_id.into()),
            leader: Arc::new(AtomicBool::new(true)),
            raft: None,
        }
    }

    pub async fn connect(
        redis_url: &str,
        node_id: impl Into<String>,
        role: &str,
    ) -> AppResult<Self> {
        Self::connect_inner(redis_url, node_id, role, true, None).await
    }

    pub async fn connect_with_raft(
        redis_url: &str,
        node_id: impl Into<String>,
        role: &str,
        raft_node_id: u64,
        raft_addr: String,
        peers: Vec<(u64, String)>,
        bootstrap: bool,
    ) -> AppResult<Self> {
        Self::connect_inner(
            redis_url,
            node_id,
            role,
            false,
            Some((raft_node_id, raft_addr, peers, bootstrap)),
        )
        .await
    }

    async fn connect_inner(
        redis_url: &str,
        node_id: impl Into<String>,
        role: &str,
        redis_leader: bool,
        raft_cfg: Option<RaftBootstrap>,
    ) -> AppResult<Self> {
        let node_id: Arc<str> = Arc::from(node_id.into());
        let client = Client::open(redis_url).map_err(|error| {
            tracing::error!(%error, "invalid Redis URL");
            AppError::internal("Redis 配置错误")
        })?;
        let mut publisher = client.get_connection_manager().await.map_err(|error| {
            tracing::error!(%error, "failed to connect Redis");
            AppError::internal("无法连接 Redis，集群模式启动失败")
        })?;
        let _: String = redis::cmd("PING")
            .query_async(&mut publisher)
            .await
            .map_err(|error| {
                tracing::error!(%error, "Redis PING failed");
                AppError::internal("Redis 健康检查失败")
            })?;

        let mut cluster = Self {
            inner: Some(Arc::new(ClusterInner {
                client: client.clone(),
                publisher,
            })),
            node_id: node_id.clone(),
            leader: Arc::new(AtomicBool::new(false)),
            raft: None,
        };
        cluster.announce_node(role).await?;
        cluster.spawn_node_heartbeat(role.to_string());
        if redis_leader && matches!(role, "api" | "all" | "worker" | "scheduler") {
            cluster.spawn_leader_election();
        } else {
            // Gateway-only nodes must not steal the singleton scheduler lease.
            cluster.leader.store(false, Ordering::Relaxed);
        }
        if let Some((raft_node_id, raft_addr, peers, bootstrap)) = raft_cfg {
            let control = RaftControl::start(
                raft_node_id,
                raft_addr,
                peers,
                bootstrap,
                cluster.leader.clone(),
            )
            .await?;
            cluster.raft = Some(Arc::new(control));
        }
        Ok(cluster)
    }

    pub fn node_id(&self) -> &str {
        &self.node_id
    }

    pub fn enabled(&self) -> bool {
        self.inner.is_some()
    }

    pub fn is_leader(&self) -> bool {
        self.leader.load(Ordering::Relaxed)
    }

    pub fn leader_flag(&self) -> Arc<AtomicBool> {
        self.leader.clone()
    }

    pub fn status(&self) -> ClusterStatus {
        let raft = self.raft.as_ref().map(|raft| raft.status());
        ClusterStatus {
            enabled: self.enabled(),
            node_id: self.node_id.to_string(),
            leader: self.is_leader(),
            redis_connected: self.inner.is_some(),
            raft_enabled: raft.is_some(),
            raft_node_id: raft.as_ref().map(|status| status.node_id),
            raft_leader: raft.as_ref().and_then(|status| status.current_leader),
            raft_term: raft.as_ref().map(|status| status.term).unwrap_or(0),
            raft_state: raft
                .as_ref()
                .map(|status| status.state.clone())
                .unwrap_or_else(|| "disabled".to_string()),
            raft_last_log: raft.as_ref().and_then(|status| status.last_log_index),
        }
    }

    fn connection(&self) -> Option<ConnectionManager> {
        self.inner.as_ref().map(|inner| inner.publisher.clone())
    }

    pub(crate) async fn revoke_token(&self, jti: &str, ttl_seconds: u64) -> AppResult<()> {
        let Some(mut connection) = self.connection() else {
            return Ok(());
        };
        let key = format!("{REVOKED_TOKEN_PREFIX}{jti}");
        redis::cmd("SETEX")
            .arg(key)
            .arg(ttl_seconds.max(1))
            .arg("1")
            .query_async::<()>(&mut connection)
            .await
            .map_err(|error| {
                tracing::warn!(%error, "Redis revoke token failed");
                AppError::internal("Redis 凭证吊销失败")
            })?;
        Ok(())
    }

    /// Read-through cache for terminal -> vehicle binding and metadata.
    /// Location/auth hot paths hit this before PostgreSQL.
    pub(crate) async fn cached_terminal(&self, phone: &str) -> Option<Terminal> {
        let mut conn = self.connection()?;
        let key = format!("{TERMINAL_CACHE_PREFIX}{phone}");
        let raw: Option<String> = conn.get(key).await.ok()?;
        serde_json::from_str(&raw?).ok()
    }

    pub(crate) async fn cache_terminal(&self, terminal: &Terminal) {
        let Some(mut conn) = self.connection() else {
            return;
        };
        let Ok(raw) = serde_json::to_string(terminal) else {
            return;
        };
        let key = format!("{TERMINAL_CACHE_PREFIX}{}", terminal.phone);
        if let Err(error) = conn
            .set_ex::<_, _, ()>(&key, raw, TERMINAL_CACHE_TTL_SECONDS)
            .await
        {
            tracing::debug!(%error, "terminal cache write failed");
        }
    }

    pub(crate) async fn invalidate_terminal(&self, phone: &str) {
        let Some(mut conn) = self.connection() else {
            return;
        };
        let key = format!("{TERMINAL_CACHE_PREFIX}{phone}");
        if let Err(error) = conn.del::<_, ()>(&key).await {
            tracing::debug!(%error, "terminal cache invalidate failed");
        }
    }

    pub(crate) async fn is_token_revoked(&self, jti: &str) -> AppResult<bool> {
        let Some(mut connection) = self.connection() else {
            return Ok(false);
        };
        let key = format!("{REVOKED_TOKEN_PREFIX}{jti}");
        let exists: i64 = redis::cmd("EXISTS")
            .arg(key)
            .query_async(&mut connection)
            .await
            .map_err(|error| {
                tracing::warn!(%error, "Redis revoked-token check failed");
                AppError::internal("Redis 凭证状态查询失败")
            })?;
        Ok(exists > 0)
    }

    async fn announce_node(&self, role: &str) -> AppResult<()> {
        let Some(mut conn) = self.connection() else {
            return Ok(());
        };
        let key = format!("tsp:node:{}", self.node_id);
        let value = serde_json::json!({
            "node_id": self.node_id.as_ref(),
            "role": role,
            "ts": crate::util::now_iso()
        })
        .to_string();
        let _: () = conn
            .set_ex(&key, value, NODE_TTL_SECONDS)
            .await
            .map_err(|error| {
                tracing::warn!(%error, "failed to announce cluster node");
                AppError::internal("集群节点注册失败")
            })?;
        Ok(())
    }

    fn spawn_node_heartbeat(&self, role: String) {
        if self.inner.is_none() {
            return;
        }
        let cluster = self.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(15));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                interval.tick().await;
                if let Err(error) = cluster.announce_node(&role).await {
                    tracing::warn!(%error, node = %cluster.node_id, "cluster node heartbeat failed");
                }
            }
        });
    }

    fn spawn_leader_election(&self) {
        if self.inner.is_none() {
            return;
        }
        let cluster = self.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(5));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                interval.tick().await;
                let Some(mut conn) = cluster.connection() else {
                    cluster.leader.store(false, Ordering::Relaxed);
                    continue;
                };
                let acquired: Option<String> = redis::cmd("SET")
                    .arg(LEADER_KEY)
                    .arg(cluster.node_id.as_ref())
                    .arg("NX")
                    .arg("PX")
                    .arg(LEADER_TTL_MS)
                    .query_async(&mut conn)
                    .await
                    .unwrap_or(None);
                if acquired.as_deref() == Some("OK") {
                    if !cluster.leader.swap(true, Ordering::Relaxed) {
                        tracing::info!(node = %cluster.node_id, "acquired scheduler leadership");
                    }
                    continue;
                }

                // Renew only if this node still owns the lease.
                let renewed: i64 = Script::new(
                    r#"
                    if redis.call('GET', KEYS[1]) == ARGV[1] then
                        return redis.call('PEXPIRE', KEYS[1], ARGV[2])
                    end
                    return 0
                    "#,
                )
                .key(LEADER_KEY)
                .arg(cluster.node_id.as_ref())
                .arg(LEADER_TTL_MS)
                .invoke_async(&mut conn)
                .await
                .unwrap_or(0);
                if renewed == 0 && cluster.leader.swap(false, Ordering::Relaxed) {
                    tracing::warn!(node = %cluster.node_id, "lost scheduler leadership");
                }
            }
        });
    }

    /// Register or refresh a terminal's gateway ownership.
    pub async fn register_session(&self, phone: &str, peer: &str) {
        let Some(mut conn) = self.connection() else {
            return;
        };
        let key = session_key(phone);
        let value = format!("{}|{}", self.node_id, peer);
        if let Err(error) = conn
            .set_ex::<_, _, ()>(&key, value, SESSION_TTL_SECONDS)
            .await
        {
            tracing::warn!(%error, phone, "failed to register gateway session");
        }
    }

    pub async fn unregister_session(&self, phone: &str, peer: &str) {
        let Some(mut conn) = self.connection() else {
            return;
        };
        let key = session_key(phone);
        let value = format!("{}|{}", self.node_id, peer);
        let removed: i64 = Script::new(
            r#"
            if redis.call('GET', KEYS[1]) == ARGV[1] then
                return redis.call('DEL', KEYS[1])
            end
            return 0
            "#,
        )
        .key(&key)
        .arg(value)
        .invoke_async(&mut conn)
        .await
        .unwrap_or(0);
        if removed > 0 {
            tracing::debug!(phone, "released gateway session");
        }
    }

    pub async fn lookup_session(&self, phone: &str) -> Option<SessionInfo> {
        let mut conn = self.connection()?;
        let value: Option<String> = conn.get(session_key(phone)).await.ok()?;
        let value = value?;
        let (node_id, peer) = value.split_once('|')?;
        Some(SessionInfo {
            node_id: node_id.to_string(),
            peer: peer.to_string(),
        })
    }

    /// Publish a message to the cluster-wide realtime channel.
    pub async fn publish_event(&self, payload: &str) -> bool {
        let Some(mut conn) = self.connection() else {
            return false;
        };
        let result: Result<i64, _> = conn.publish(EVENTS_CHANNEL, payload).await;
        match result {
            Ok(_) => true,
            Err(error) => {
                tracing::warn!(%error, "failed to publish cluster event");
                false
            }
        }
    }

    /// Route a downlink frame to the gateway replica that owns the terminal.
    pub async fn publish_gateway(&self, node_id: &str, payload: &str) -> bool {
        let Some(mut conn) = self.connection() else {
            return false;
        };
        let channel = format!("{GATEWAY_CHANNEL_PREFIX}{node_id}");
        let result: Result<i64, _> = conn.publish(channel, payload).await;
        match result {
            Ok(_) => true,
            Err(error) => {
                tracing::warn!(%error, "failed to publish gateway command");
                false
            }
        }
    }

    /// Ask other gateway replicas to close a terminal's old TCP connection
    /// after a new session claims the same phone.
    pub async fn publish_jt808_kick(&self, phone: &str, owner_node: &str) -> bool {
        let Some(mut conn) = self.connection() else {
            return false;
        };
        let payload = serde_json::json!({"phone": phone, "node_id": owner_node}).to_string();
        let result: Result<i64, _> = conn.publish(JT808_KICK_CHANNEL, payload).await;
        match result {
            Ok(_) => true,
            Err(error) => {
                tracing::warn!(%error, "failed to publish JT808 kick");
                false
            }
        }
    }

    pub fn spawn_jt808_kick_subscriber<F>(&self, mut handler: F)
    where
        F: FnMut(String) + Send + 'static,
    {
        let Some(inner) = self.inner.clone() else {
            return;
        };
        let node_id = self.node_id.to_string();
        tokio::spawn(async move {
            loop {
                match inner.client.get_async_pubsub().await {
                    Ok(mut pubsub) => {
                        if let Err(error) = pubsub.subscribe(JT808_KICK_CHANNEL).await {
                            tracing::warn!(%error, "failed to subscribe JT808 kick channel");
                            tokio::time::sleep(Duration::from_secs(3)).await;
                            continue;
                        }
                        tracing::info!(node = %node_id, "JT808 kick subscriber ready");
                        let mut stream = pubsub.on_message();
                        while let Some(message) = stream.next().await {
                            let Ok(payload) = message.get_payload::<String>() else {
                                continue;
                            };
                            handler(payload);
                        }
                        tracing::warn!("JT808 kick subscriber reconnect");
                        tokio::time::sleep(Duration::from_secs(3)).await;
                    }
                    Err(error) => {
                        tracing::warn!(%error, "JT808 kick subscriber connection failed");
                        tokio::time::sleep(Duration::from_secs(3)).await;
                    }
                }
            }
        });
    }

    /// Subscribe to this replica's gateway channel. Gateway replicas receive
    /// 0x8300 frames published by an API replica that does not own the socket.
    pub fn spawn_gateway_subscriber<F>(&self, mut handler: F)
    where
        F: FnMut(String) + Send + 'static,
    {
        let Some(inner) = self.inner.clone() else {
            return;
        };
        let node_id = self.node_id.to_string();
        let channel = format!("{GATEWAY_CHANNEL_PREFIX}{node_id}");
        tokio::spawn(async move {
            loop {
                match inner.client.get_async_pubsub().await {
                    Ok(mut pubsub) => {
                        if let Err(error) = pubsub.subscribe(&channel).await {
                            tracing::warn!(%error, "failed to subscribe gateway channel");
                            tokio::time::sleep(Duration::from_secs(3)).await;
                            continue;
                        }
                        tracing::info!(node = %node_id, %channel, "gateway command subscriber ready");
                        let mut stream = pubsub.on_message();
                        while let Some(message) = stream.next().await {
                            let Ok(payload) = message.get_payload::<String>() else {
                                continue;
                            };
                            handler(payload);
                        }
                    }
                    Err(error) => {
                        tracing::warn!(%error, "gateway subscriber reconnect");
                        tokio::time::sleep(Duration::from_secs(3)).await;
                    }
                }
            }
        });
    }

    /// Start a Redis Pub/Sub listener. The callback runs on the async runtime
    /// and must not block.
    pub fn spawn_event_subscriber<F>(&self, mut handler: F)
    where
        F: FnMut(String) + Send + 'static,
    {
        let Some(inner) = self.inner.clone() else {
            return;
        };
        let node_id = self.node_id.to_string();
        tokio::spawn(async move {
            loop {
                match inner.client.get_async_pubsub().await {
                    Ok(mut pubsub) => {
                        if let Err(error) = pubsub.subscribe(EVENTS_CHANNEL).await {
                            tracing::warn!(%error, "failed to subscribe cluster events");
                            tokio::time::sleep(Duration::from_secs(3)).await;
                            continue;
                        }
                        tracing::info!(node = %node_id, "cluster event subscriber ready");
                        let mut stream = pubsub.on_message();
                        while let Some(message) = stream.next().await {
                            let Ok(payload) = message.get_payload::<String>() else {
                                continue;
                            };
                            handler(payload);
                        }
                    }
                    Err(error) => {
                        tracing::warn!(%error, "cluster event subscriber connection failed");
                        tokio::time::sleep(Duration::from_secs(3)).await;
                    }
                }
            }
        });
    }
}

fn session_key(phone: &str) -> String {
    format!("tsp:session:{phone}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_cluster_is_always_leader() {
        let cluster = Cluster::disabled("test-node");
        assert!(!cluster.enabled());
        assert!(cluster.is_leader());
        assert_eq!(cluster.node_id(), "test-node");
        assert_eq!(cluster.status().node_id, "test-node");
    }

    #[test]
    fn session_key_is_namespaced() {
        assert_eq!(session_key("013800000001"), "tsp:session:013800000001");
    }

    #[tokio::test]
    async fn disabled_terminal_cache_is_fail_open() {
        let cluster = Cluster::disabled("test-api");
        assert!(cluster.cached_terminal("013800000001").await.is_none());
        cluster.invalidate_terminal("013800000001").await;
    }

    #[tokio::test]
    async fn redis_session_roundtrip() {
        let Ok(url) = std::env::var("TSP_TEST_REDIS_URL") else {
            return;
        };
        let cluster = Cluster::connect(&url, "test-api", "api")
            .await
            .expect("redis cluster");
        let phone = format!("test-{}", uuid::Uuid::new_v4());
        cluster.register_session(&phone, "peer-node").await;
        let session = cluster.lookup_session(&phone).await;
        assert_eq!(
            session.map(|session| session.peer),
            Some("peer-node".to_string())
        );
    }
}
