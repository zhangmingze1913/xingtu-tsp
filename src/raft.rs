//! OpenRaft control plane.
//!
//! OpenRaft owns only cluster metadata and the scheduler leader lease. Business
//! rows stay in PostgreSQL/MySQL, gateway sessions stay in Redis, and domain
//! events stay on Redis/Iggy/Walrus. The Raft state machine is intentionally
//! small and in-memory; losing it must not lose TSP business data.
//!
//! Wire protocol: JSON over HTTP, one listener per node (`TSP_RAFT_ADDR`).
//! Peers are configured as `id@host:port,id@host:port`.

use std::{
    collections::{BTreeMap, HashMap},
    io::Cursor,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, RwLock,
    },
};

use axum::{extract::State, routing::post, Json, Router};
use openraft::{
    async_runtime::WatchReceiver,
    errors::{NetworkError, RPCError, RaftError, ReplicationClosed, StreamingError, Unreachable},
    network::{v2::RaftNetworkV2, RPCOption, RaftNetworkFactory},
    raft::{
        AppendEntriesRequest, AppendEntriesResponse, SnapshotResponse, TransferLeaderRequest,
        TransferLeaderResponse, VoteRequest, VoteResponse,
    },
    type_config::alias::{SnapshotMetaOf, SnapshotOf, VoteOf},
    RaftTypeConfig,
};
use openraft_memstore::TypeConfig;
use serde::{de::DeserializeOwned, Serialize};
use tokio::{net::TcpListener, sync::oneshot};

use crate::error::{AppError, AppResult};
use crate::raft_log::FileLogStore;
use crate::raft_state::FileStateMachine;

type Raft = openraft::Raft<TypeConfig, FileStateMachine>;

#[derive(Clone, Debug, Default, Serialize)]
pub struct RaftStatus {
    pub node_id: u64,
    pub leader: bool,
    pub current_leader: Option<u64>,
    pub term: u64,
    pub state: String,
    pub last_log_index: Option<u64>,
    pub last_applied: Option<u64>,
}

pub struct RaftControl {
    status: Arc<RwLock<RaftStatus>>,
    _raft: Arc<Raft>,
}

impl RaftControl {
    pub async fn start(
        node_id: u64,
        raft_addr: String,
        peers: Vec<(u64, String)>,
        bootstrap: bool,
        leader_flag: Arc<AtomicBool>,
    ) -> AppResult<Self> {
        let mut peer_map: HashMap<u64, String> = peers.into_iter().collect();
        if peer_map.is_empty() {
            peer_map.insert(node_id, raft_addr.clone());
        }
        peer_map.entry(node_id).or_insert_with(|| raft_addr.clone());

        let config = openraft::Config {
            heartbeat_interval: 300,
            election_timeout_min: 900,
            election_timeout_max: 1800,
            enable_pre_vote: Some(true),
            ..Default::default()
        };
        let config = Arc::new(config.validate().map_err(|error| {
            tracing::error!(%error, "invalid OpenRaft config");
            AppError::internal("OpenRaft 配置错误")
        })?);

        let data_dir =
            std::env::var("TSP_RAFT_DATA_DIR").unwrap_or_else(|_| format!("./raft-data/{node_id}"));
        let log_store = FileLogStore::open(std::path::Path::new(&data_dir)).map_err(|error| {
            tracing::error!(%error, %data_dir, "OpenRaft log store open failed");
            AppError::internal("OpenRaft 日志存储初始化失败")
        })?;
        let state_machine =
            FileStateMachine::open(std::path::Path::new(&data_dir)).map_err(|error| {
                tracing::error!(%error, %data_dir, "OpenRaft state store open failed");
                AppError::internal("OpenRaft 状态存储初始化失败")
            })?;
        let network = NetworkFactory {
            peers: peer_map.clone(),
            client: reqwest::Client::builder()
                .no_proxy()
                .timeout(std::time::Duration::from_secs(5))
                .build()
                .map_err(|_| AppError::internal("OpenRaft HTTP 客户端初始化失败"))?,
        };
        let raft = Arc::new(
            openraft::Raft::new(node_id, config, network, log_store, state_machine)
                .await
                .map_err(|error| {
                    tracing::error!(%error, "OpenRaft start failed");
                    AppError::internal("OpenRaft 启动失败")
                })?,
        );

        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
        let server_raft = raft.clone();
        let server_addr = raft_addr.clone();
        tokio::spawn(async move {
            if let Err(error) = serve_rpc(server_raft, server_addr, shutdown_rx).await {
                tracing::error!(%error, "OpenRaft RPC server stopped");
            }
        });
        // Keep the sender alive for the process lifetime; dropping it is a
        // clean shutdown hook for the RPC listener.
        std::mem::forget(shutdown_tx);

        let status = Arc::new(RwLock::new(RaftStatus {
            node_id,
            ..Default::default()
        }));

        let members: BTreeMap<u64, ()> = peer_map.keys().map(|id| (*id, ())).collect();
        if bootstrap {
            match raft.initialize(members).await {
                Ok(()) => tracing::info!(node_id, "OpenRaft cluster initialized"),
                Err(error) => tracing::warn!(%error, node_id, "OpenRaft initialize skipped"),
            }
        }

        let metrics_raft = raft.clone();
        let metrics_status = status.clone();
        let metrics_leader = leader_flag;
        tokio::spawn(async move {
            let mut rx = metrics_raft.metrics();
            loop {
                if rx.changed().await.is_err() {
                    break;
                }
                let m = rx.borrow_watched().clone();
                let is_leader = m.current_leader == Some(node_id);
                metrics_leader.store(is_leader, Ordering::Relaxed);
                if let Ok(mut current) = metrics_status.write() {
                    *current = RaftStatus {
                        node_id,
                        leader: is_leader,
                        current_leader: m.current_leader,
                        term: m.current_term,
                        state: format!("{:?}", m.state),
                        last_log_index: m.last_log_index,
                        last_applied: m.last_applied.map(|log| log.index),
                    };
                }
            }
        });

        tracing::info!(%node_id, %raft_addr, peers = ?peer_map, "OpenRaft control plane ready");
        Ok(Self {
            status,
            _raft: raft,
        })
    }

    pub fn status(&self) -> RaftStatus {
        self.status
            .read()
            .map(|status| status.clone())
            .unwrap_or_default()
    }
}

async fn serve_rpc(
    raft: Arc<Raft>,
    addr: String,
    mut shutdown: oneshot::Receiver<()>,
) -> AppResult<()> {
    let app = Router::new()
        .route("/raft/append", post(rpc_append))
        .route("/raft/vote", post(rpc_vote))
        .route("/raft/pre-vote", post(rpc_pre_vote))
        .route("/raft/snapshot", post(rpc_snapshot))
        .route("/raft/transfer-leader", post(rpc_transfer_leader))
        .with_state(raft);
    let listener = TcpListener::bind(&addr).await.map_err(|error| {
        tracing::error!(%error, %addr, "OpenRaft bind failed");
        AppError::internal(format!("无法监听 OpenRaft RPC {addr}"))
    })?;
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            let _ = (&mut shutdown).await;
        })
        .await
        .map_err(|error| AppError::internal(format!("OpenRaft RPC 服务失败: {error}")))
}

async fn rpc_append(
    State(raft): State<Arc<Raft>>,
    Json(request): Json<AppendEntriesRequest<TypeConfig>>,
) -> Json<Result<AppendEntriesResponse<TypeConfig>, RaftError<TypeConfig>>> {
    Json(
        raft.append_entries(request)
            .await
            .map(AppendEntriesResponse::from),
    )
}

async fn rpc_vote(
    State(raft): State<Arc<Raft>>,
    Json(request): Json<VoteRequest<TypeConfig>>,
) -> Json<Result<VoteResponse<TypeConfig>, RaftError<TypeConfig>>> {
    Json(raft.vote(request).await)
}

async fn rpc_pre_vote(
    State(raft): State<Arc<Raft>>,
    Json(request): Json<VoteRequest<TypeConfig>>,
) -> Json<Result<VoteResponse<TypeConfig>, RaftError<TypeConfig>>> {
    Json(raft.pre_vote(request).await)
}

async fn rpc_snapshot(
    State(raft): State<Arc<Raft>>,
    Json((vote, meta, data)): Json<(VoteOf<TypeConfig>, SnapshotMetaOf<TypeConfig>, Vec<u8>)>,
) -> Json<Result<SnapshotResponse<TypeConfig>, RaftError<TypeConfig>>> {
    let snapshot = openraft::Snapshot {
        meta,
        snapshot: Cursor::new(data),
    };
    Json(
        raft.install_full_snapshot(vote, snapshot)
            .await
            .map_err(RaftError::Fatal),
    )
}

async fn rpc_transfer_leader(
    State(raft): State<Arc<Raft>>,
    Json(request): Json<TransferLeaderRequest<TypeConfig>>,
) -> Json<Result<TransferLeaderResponse<TypeConfig>, RaftError<TypeConfig>>> {
    Json(
        raft.handle_transfer_leader(request)
            .await
            .map_err(RaftError::Fatal),
    )
}

struct NetworkFactory {
    peers: HashMap<u64, String>,
    client: reqwest::Client,
}

struct RaftClient {
    addr: String,
    client: reqwest::Client,
}

impl RaftNetworkFactory<TypeConfig> for NetworkFactory {
    type Network = RaftClient;

    async fn new_client(&mut self, target: u64, _node: &()) -> Self::Network {
        let addr = self
            .peers
            .get(&target)
            .cloned()
            .unwrap_or_else(|| format!("127.0.0.1:{}", 7000 + target));
        RaftClient {
            addr,
            client: self.client.clone(),
        }
    }
}

impl RaftClient {
    async fn request<C, Req, Resp>(
        &self,
        path: &str,
        request: Req,
        option: &RPCOption,
    ) -> Result<Result<Resp, RaftError<C>>, RPCError<C>>
    where
        C: RaftTypeConfig,
        Req: Serialize,
        Result<Resp, RaftError<C>>: DeserializeOwned,
    {
        let url = format!("http://{}/raft/{path}", self.addr);
        let response = self
            .client
            .post(&url)
            .json(&request)
            .timeout(option.soft_ttl())
            .send()
            .await
            .map_err(|error| {
                if error.is_connect() {
                    RPCError::Unreachable(Unreachable::new(&error))
                } else {
                    RPCError::Network(NetworkError::new(&error))
                }
            })?;
        if !response.status().is_success() {
            return Err(RPCError::Network(NetworkError::from_string(format!(
                "HTTP {} from {url}",
                response.status()
            ))));
        }
        response
            .json()
            .await
            .map_err(|error| RPCError::Network(NetworkError::new(&error)))
    }
}

impl RaftNetworkV2<TypeConfig> for RaftClient {
    type SnapshotData = Cursor<Vec<u8>>;

    async fn append_entries(
        &mut self,
        request: AppendEntriesRequest<TypeConfig>,
        option: RPCOption,
    ) -> Result<openraft::raft::AppendResult<TypeConfig>, RPCError<TypeConfig>> {
        use openraft::entry::RaftEntry;
        let prev_log_id = request.prev_log_id;
        // A heartbeat has no entries: `Success` acknowledges `prev_log_id`.
        let last_log_id = request
            .entries
            .last()
            .map(|entry| entry.log_id())
            .or(prev_log_id);
        let response: Result<AppendEntriesResponse<TypeConfig>, RaftError<TypeConfig>> =
            self.request("append", request, &option).await?;
        let response = response.map_err(|error| RPCError::Unreachable(Unreachable::new(&error)))?;
        Ok(response.into_append_result(prev_log_id, last_log_id))
    }

    async fn full_snapshot(
        &mut self,
        vote: VoteOf<TypeConfig>,
        snapshot: SnapshotOf<TypeConfig, Self::SnapshotData>,
        cancel: impl std::future::Future<Output = ReplicationClosed> + openraft::OptionalSend + 'static,
        option: RPCOption,
    ) -> Result<SnapshotResponse<TypeConfig>, StreamingError<TypeConfig>> {
        let request = (vote, snapshot.meta, snapshot.snapshot.into_inner());
        tokio::pin!(cancel);
        tokio::select! {
            closed = &mut cancel => Err(StreamingError::Closed(closed)),
            result = self.request("snapshot", request, &option) => {
                let response = result?;
                response.map_err(|error| {
                    StreamingError::Unreachable(Unreachable::new(&error))
                })
            }
        }
    }

    async fn transfer_leader(
        &mut self,
        request: TransferLeaderRequest<TypeConfig>,
        option: RPCOption,
    ) -> Result<TransferLeaderResponse<TypeConfig>, RPCError<TypeConfig>> {
        let response: Result<TransferLeaderResponse<TypeConfig>, RaftError<TypeConfig>> =
            self.request("transfer-leader", request, &option).await?;
        response.map_err(|error| RPCError::Unreachable(Unreachable::new(&error)))
    }

    async fn vote(
        &mut self,
        request: VoteRequest<TypeConfig>,
        option: RPCOption,
    ) -> Result<VoteResponse<TypeConfig>, RPCError<TypeConfig>> {
        let response: Result<VoteResponse<TypeConfig>, RaftError<TypeConfig>> =
            self.request("vote", request, &option).await?;
        response.map_err(|error| RPCError::Unreachable(Unreachable::new(&error)))
    }

    async fn pre_vote(
        &mut self,
        request: VoteRequest<TypeConfig>,
        option: RPCOption,
    ) -> Result<VoteResponse<TypeConfig>, RPCError<TypeConfig>> {
        let response: Result<VoteResponse<TypeConfig>, RaftError<TypeConfig>> =
            self.request("pre-vote", request, &option).await?;
        response.map_err(|error| RPCError::Unreachable(Unreachable::new(&error)))
    }
}

pub fn parse_peers(raw: &str) -> Vec<(u64, String)> {
    raw.split(',')
        .filter_map(|part| {
            let part = part.trim();
            if part.is_empty() {
                return None;
            }
            if let Some((id, addr)) = part.split_once('@') {
                if addr.is_empty() {
                    return None;
                }
                return Some((id.parse().ok()?, addr.to_string()));
            }
            // StatefulSet form: host:port or host.namespace:port. Derive the
            // id from the stable first host label so `TSP_NODE_ID`=pod name
            // and the peer list agree without per-pod environment injection.
            let host = part.split(':').next()?;
            let short = host.split('.').next()?;
            if short.is_empty() || !part.contains(':') {
                return None;
            }
            Some((crate::config::stable_node_id(short), part.to_string()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_raft_peer_list() {
        let peers =
            parse_peers("1@127.0.0.1:7001, 2@10.0.0.2:7001,bad,3@,tsp-api-0.tsp-api-headless:7001");
        assert_eq!(
            peers,
            vec![
                (1, "127.0.0.1:7001".to_string()),
                (2, "10.0.0.2:7001".to_string()),
                (
                    crate::config::stable_node_id("tsp-api-0"),
                    "tsp-api-0.tsp-api-headless:7001".to_string()
                )
            ]
        );
    }
}
