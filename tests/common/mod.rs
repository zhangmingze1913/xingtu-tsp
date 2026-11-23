#![allow(dead_code)]

use axum::{
    body::Body,
    http::{header, Method, Request, StatusCode},
    Router,
};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

use xingtu_tsp::{auth::hash_password, build_app, config::Config, db::Db, state::AppState};

pub fn test_config() -> Config {
    Config {
        bind: "127.0.0.1:0".to_string(),
        database_url: "sqlite::memory:".to_string(),
        database_path: std::path::PathBuf::from(":memory:"),
        web_dir: std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("web"),
        jwt_private_key: None,
        jwt_public_key: None,
        jwt_key_id: "test-key".to_string(),
        jwt_issuer: "xingtu-test".to_string(),
        jwt_audience: "xingtu-test-api".to_string(),
        jwt_access_ttl_minutes: 5,
        jwt_refresh_ttl_hours: 1,
        jwt_previous_keys: Vec::new(),
        jt808_addr: "127.0.0.1:0".to_string(),
        demo_mode: false,
        cluster_mode: false,
        redis_url: None,
        node_id: "test-node".to_string(),
        node_role: "api".to_string(),
        iggy_url: None,
        iggy_stream: "xingtu-tsp".to_string(),
        iggy_topic: "events".to_string(),
        iggy_username: "iggy".to_string(),
        iggy_password: "iggy".to_string(),
        tsink_url: None,
        tsink_token: None,
        tdengine_url: None,
        tdengine_user: "root".to_string(),
        tdengine_password: "taosdata".to_string(),
        tdengine_database: "tsp".to_string(),
        walrus_addr: None,
        walrus_topic: "tsp-events".to_string(),
        raft_enabled: false,
        raft_node_id: 1,
        raft_addr: "127.0.0.1:7001".to_string(),
        raft_peers: String::new(),
        raft_bootstrap: true,
        production: false,
        cors_origins: Vec::new(),
        admin_password: None,
        command_mode: "simulated".to_string(),
        postgis_enabled: true,
        redis_allow_plaintext: false,
        jt808_tls_cert: None,
        jt808_tls_key: None,
        jt808_allow_plaintext: true,
        jt808_client_ca: None,
        jt808_allowed_cidrs: Vec::new(),
    }
}

pub async fn setup() -> (Router, AppState) {
    let db = Db::in_memory().await.expect("in-memory db");
    let hash = hash_password("admin123").expect("hash");
    db.create_user("admin", &hash, "测试管理员", "admin", "")
        .await
        .expect("user");
    let state = AppState::new(test_config(), db);
    let app = build_app(state.clone());
    (app, state)
}

pub async fn request(
    app: &Router,
    method: Method,
    uri: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(token) = token {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    let request = if let Some(body) = body {
        builder
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string()))
            .expect("request")
    } else {
        builder.body(Body::empty()).expect("request")
    };
    let response = app.clone().oneshot(request).await.expect("response");
    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, value)
}

#[allow(dead_code)]
pub async fn login(app: &Router) -> String {
    let (status, body) = request(
        app,
        Method::POST,
        "/api/auth/login",
        None,
        Some(json!({"username": "admin", "password": "admin123"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "login failed: {body}");
    body["token"].as_str().expect("token").to_string()
}

pub async fn request_text(
    app: &Router,
    method: Method,
    uri: &str,
    token: Option<&str>,
) -> (StatusCode, String) {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(token) = token {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    let response = app
        .clone()
        .oneshot(builder.body(Body::empty()).expect("request"))
        .await
        .expect("response");
    let status = response.status();
    let body = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    (status, String::from_utf8_lossy(&body).to_string())
}
