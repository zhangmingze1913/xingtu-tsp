mod common;

use axum::{http::Method, http::StatusCode};

#[tokio::test]
async fn statefulset_peer_dns_parses() {
    let peers =
        xingtu_tsp::parse_peers("tsp-api-0.tsp-api-headless:7001,tsp-api-1.tsp-api-headless:7001");
    assert_eq!(peers.len(), 2);
    assert!(peers
        .iter()
        .all(|(id, addr)| *id > 0 && addr.ends_with(":7001")));
    let (status, _) = common::request(
        &common::setup().await.0,
        Method::GET,
        "/api/health",
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}
