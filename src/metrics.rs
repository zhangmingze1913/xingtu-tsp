use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};

use axum::{
    extract::{Request, State},
    middleware::Next,
    response::Response,
};

use crate::{cluster::ClusterStatus, state::AppState};

#[derive(Default)]
pub struct Metrics {
    pub http_requests: AtomicU64,
    pub telemetry_ingested: AtomicU64,
    pub alerts_created: AtomicU64,
    pub commands_created: AtomicU64,
    pub ws_connections: AtomicI64,
    pub jt808_connections: AtomicI64,
    pub jt808_frames: AtomicU64,
    pub jt808_frame_errors: AtomicU64,
    pub jt808_auth_failures: AtomicU64,
    pub telemetry_duplicates: AtomicU64,
    pub offline_sweeps: AtomicU64,
    pub jt808_downlinks: AtomicU64,
}

impl Metrics {
    pub fn render(&self, cluster: &ClusterStatus) -> String {
        let mut out = String::with_capacity(2048);
        out.push_str("# HELP xingtu_http_requests_total Total HTTP requests.\n");
        out.push_str("# TYPE xingtu_http_requests_total counter\n");
        out.push_str(&format!(
            "xingtu_http_requests_total {}\n",
            self.http_requests.load(Ordering::Relaxed)
        ));
        out.push_str("# HELP xingtu_telemetry_ingested_total Accepted location samples.\n");
        out.push_str("# TYPE xingtu_telemetry_ingested_total counter\n");
        out.push_str(&format!(
            "xingtu_telemetry_ingested_total {}\n",
            self.telemetry_ingested.load(Ordering::Relaxed)
        ));
        out.push_str("# HELP xingtu_alerts_created_total Alerts created by rules or devices.\n");
        out.push_str("# TYPE xingtu_alerts_created_total counter\n");
        out.push_str(&format!(
            "xingtu_alerts_created_total {}\n",
            self.alerts_created.load(Ordering::Relaxed)
        ));
        out.push_str("# HELP xingtu_commands_created_total Remote commands accepted.\n");
        out.push_str("# TYPE xingtu_commands_created_total counter\n");
        out.push_str(&format!(
            "xingtu_commands_created_total {}\n",
            self.commands_created.load(Ordering::Relaxed)
        ));
        out.push_str("# HELP xingtu_ws_connections Current WebSocket connections.\n");
        out.push_str("# TYPE xingtu_ws_connections gauge\n");
        out.push_str(&format!(
            "xingtu_ws_connections {}\n",
            self.ws_connections.load(Ordering::Relaxed)
        ));
        out.push_str("# HELP xingtu_jt808_connections Current JT/T 808 TCP connections.\n");
        out.push_str("# TYPE xingtu_jt808_connections gauge\n");
        out.push_str(&format!(
            "xingtu_jt808_connections {}\n",
            self.jt808_connections.load(Ordering::Relaxed)
        ));
        out.push_str("# HELP xingtu_jt808_frames_total JT/T 808 frames parsed.\n");
        out.push_str("# TYPE xingtu_jt808_frames_total counter\n");
        out.push_str(&format!(
            "xingtu_jt808_frames_total {}\n",
            self.jt808_frames.load(Ordering::Relaxed)
        ));
        out.push_str("# HELP xingtu_jt808_frame_errors_total JT/T 808 parse/checksum errors.\n");
        out.push_str("# TYPE xingtu_jt808_frame_errors_total counter\n");
        out.push_str(&format!(
            "xingtu_jt808_frame_errors_total {}\n",
            self.jt808_frame_errors.load(Ordering::Relaxed)
        ));
        out.push_str("# HELP xingtu_jt808_auth_failures_total JT/T 808 authentication failures.\n");
        out.push_str("# TYPE xingtu_jt808_auth_failures_total counter\n");
        out.push_str(&format!(
            "xingtu_jt808_auth_failures_total {}\n",
            self.jt808_auth_failures.load(Ordering::Relaxed)
        ));
        out.push_str("# HELP xingtu_telemetry_duplicates_total Duplicate vehicle/timestamp samples rejected.\n");
        out.push_str("# TYPE xingtu_telemetry_duplicates_total counter\n");
        out.push_str(&format!(
            "xingtu_telemetry_duplicates_total {}\n",
            self.telemetry_duplicates.load(Ordering::Relaxed)
        ));
        out.push_str("# HELP xingtu_offline_sweeps_total Offline sweep runs that changed state.\n");
        out.push_str("# TYPE xingtu_offline_sweeps_total counter\n");
        out.push_str(&format!(
            "xingtu_offline_sweeps_total {}\n",
            self.offline_sweeps.load(Ordering::Relaxed)
        ));
        out.push_str(
            "# HELP xingtu_jt808_downlinks_total JT/T 808 downlink frames sent by APIs.\n",
        );
        out.push_str("# TYPE xingtu_jt808_downlinks_total counter\n");
        out.push_str(&format!(
            "xingtu_jt808_downlinks_total {}\n",
            self.jt808_downlinks.load(Ordering::Relaxed)
        ));
        let archive = crate::media::archive::stats();
        out.push_str(
            "# HELP xingtu_archive_upload_success_total RustFS segment uploads completed.\n",
        );
        out.push_str("# TYPE xingtu_archive_upload_success_total counter\n");
        out.push_str(&format!(
            "xingtu_archive_upload_success_total {}\n",
            archive.upload_success
        ));
        out.push_str("# HELP xingtu_archive_upload_failure_total RustFS segment uploads failed after retries.\n");
        out.push_str("# TYPE xingtu_archive_upload_failure_total counter\n");
        out.push_str(&format!(
            "xingtu_archive_upload_failure_total {}\n",
            archive.upload_failure
        ));
        out.push_str("# HELP xingtu_archive_upload_retries_total RustFS upload retry attempts.\n");
        out.push_str("# TYPE xingtu_archive_upload_retries_total counter\n");
        out.push_str(&format!(
            "xingtu_archive_upload_retries_total {}\n",
            archive.upload_retries
        ));
        out.push_str("# HELP xingtu_archive_segments_dropped_total Media segments dropped without a keyframe.\n");
        out.push_str("# TYPE xingtu_archive_segments_dropped_total counter\n");
        out.push_str(&format!(
            "xingtu_archive_segments_dropped_total {}\n",
            archive.segments_dropped
        ));
        out.push_str(
            "# HELP xingtu_archive_cleanup_deleted_total Expired RustFS objects deleted.\n",
        );
        out.push_str("# TYPE xingtu_archive_cleanup_deleted_total counter\n");
        out.push_str(&format!(
            "xingtu_archive_cleanup_deleted_total {}\n",
            archive.cleanup_deleted
        ));
        out.push_str("# HELP xingtu_archive_playback_success_total Archived segments remuxed for playback.\n");
        out.push_str("# TYPE xingtu_archive_playback_success_total counter\n");
        out.push_str(&format!(
            "xingtu_archive_playback_success_total {}\n",
            archive.playback_success
        ));
        out.push_str(
            "# HELP xingtu_archive_playback_failure_total Archived segment playback failures.\n",
        );
        out.push_str("# TYPE xingtu_archive_playback_failure_total counter\n");
        out.push_str(&format!(
            "xingtu_archive_playback_failure_total {}\n",
            archive.playback_failure
        ));
        out.push_str(
            "# HELP xingtu_archive_playback_cache_hit_total Archive playbacks served from cache.\n",
        );
        out.push_str("# TYPE xingtu_archive_playback_cache_hit_total counter\n");
        out.push_str(&format!(
            "xingtu_archive_playback_cache_hit_total {}\n",
            archive.playback_cache_hit
        ));
        out.push_str("# HELP xingtu_archive_playback_cache_miss_total Archive playbacks that required remux.\n");
        out.push_str("# TYPE xingtu_archive_playback_cache_miss_total counter\n");
        out.push_str(&format!(
            "xingtu_archive_playback_cache_miss_total {}\n",
            archive.playback_cache_miss
        ));
        out.push_str("# HELP xingtu_cluster_leader Whether this node owns scheduler leadership.\n");
        out.push_str("# TYPE xingtu_cluster_leader gauge\n");
        out.push_str(&format!(
            "xingtu_cluster_leader {}\n",
            if cluster.leader { 1 } else { 0 }
        ));
        out.push_str(
            "# HELP xingtu_cluster_enabled Whether Redis cluster coordination is enabled.\n",
        );
        out.push_str("# TYPE xingtu_cluster_enabled gauge\n");
        out.push_str(&format!(
            "xingtu_cluster_enabled {}\n",
            if cluster.enabled { 1 } else { 0 }
        ));
        out
    }
}

pub async fn count_http(State(state): State<AppState>, request: Request, next: Next) -> Response {
    state.metrics.http_requests.fetch_add(1, Ordering::Relaxed);
    next.run(request).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_includes_archive_counters() {
        let metrics = Metrics::default();
        let cluster = ClusterStatus {
            enabled: false,
            node_id: "test".to_string(),
            leader: false,
            redis_connected: false,
            raft_enabled: false,
            raft_node_id: None,
            raft_leader: None,
            raft_term: 0,
            raft_state: "idle".to_string(),
            raft_last_log: None,
        };
        let text = metrics.render(&cluster);
        assert!(text.contains("xingtu_archive_upload_success_total"));
        assert!(text.contains("xingtu_archive_segments_dropped_total"));
        assert!(text.contains("xingtu_archive_playback_failure_total"));
        assert!(text.contains("xingtu_archive_playback_cache_hit_total"));
    }
}
