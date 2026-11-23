use crate::{
    cluster::Cluster,
    config::Config,
    error::{AppError, AppResult},
};

pub(super) async fn connect(config: &Config) -> AppResult<Cluster> {
    if !config.cluster_mode {
        if config.raft_enabled {
            return Err(AppError::internal(
                "TSP_RAFT_ENABLED 仅在 TSP_CLUSTER_MODE=true 时生效",
            ));
        }
        return Ok(Cluster::disabled(&config.node_id));
    }
    let redis_url = config
        .redis_url
        .as_deref()
        .ok_or_else(|| AppError::internal("集群模式需要配置 TSP_REDIS_URL"))?;
    if config.raft_enabled {
        let peers = crate::raft::parse_peers(&config.raft_peers);
        let cluster = Cluster::connect_with_raft(
            redis_url,
            &config.node_id,
            &config.node_role,
            config.raft_node_id,
            config.raft_addr.clone(),
            peers,
            config.raft_bootstrap,
        )
        .await?;
        tracing::info!(
            node = %config.node_id,
            role = %config.node_role,
            raft_node = config.raft_node_id,
            "cluster coordination enabled (Redis + OpenRaft)"
        );
        Ok(cluster)
    } else {
        let cluster = Cluster::connect(redis_url, &config.node_id, &config.node_role).await?;
        tracing::info!(
            node = %config.node_id,
            role = %config.node_role,
            "cluster coordination enabled (Redis)"
        );
        Ok(cluster)
    }
}
