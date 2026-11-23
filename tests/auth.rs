mod common;

use axum::{http::Method, http::StatusCode};
use serde_json::json;
use xingtu_tsp::auth::hash_password;

use common::{request, setup};

#[tokio::test]
async fn authentication_is_required() {
    let (app, _) = setup().await;
    let (status, body) = request(&app, Method::GET, "/api/vehicles", None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"]["code"], "unauthorized");

    let (status, _) = request(
        &app,
        Method::POST,
        "/api/auth/login",
        None,
        Some(json!({"username": "admin", "password": "wrong"})),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn viewer_cannot_write() {
    let (app, state) = setup().await;
    let hash = hash_password("viewer123").expect("hash");
    state
        .db
        .create_user("viewer", &hash, "只读用户", "viewer", "")
        .await
        .expect("viewer");
    let (status, body) = request(
        &app,
        Method::POST,
        "/api/auth/login",
        None,
        Some(json!({"username": "viewer", "password": "viewer123"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let token = body["token"].as_str().expect("token");
    let (status, body) = request(
        &app,
        Method::POST,
        "/api/vehicles",
        Some(token),
        Some(json!({
            "vin": "LSVAA2A41NX888888",
            "plate_no": "沪A·T8888",
            "model": "只读测试",
            "energy_type": "EV"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["error"]["code"], "forbidden");
}

#[tokio::test]
async fn rs256_jwks_refresh_rotation_and_logout_revoke_work() {
    let (app, _) = setup().await;
    let (status, jwks) = request(&app, Method::GET, "/.well-known/jwks.json", None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(jwks["keys"][0]["alg"], "RS256");
    assert!(jwks["keys"][0]["n"]
        .as_str()
        .is_some_and(|value| !value.is_empty()));

    let (status, body) = request(
        &app,
        Method::POST,
        "/api/auth/login",
        None,
        Some(json!({"username": "admin", "password": "admin123"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let access = body["access_token"].as_str().expect("access").to_string();
    let refresh = body["refresh_token"].as_str().expect("refresh").to_string();

    let (status, refreshed) = request(
        &app,
        Method::POST,
        "/api/auth/refresh",
        None,
        Some(json!({"refresh_token": refresh.clone()})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let rotated = refreshed["refresh_token"]
        .as_str()
        .expect("rotated")
        .to_string();
    assert_ne!(refresh, rotated);

    // Rotation revokes the old refresh token.
    let (status, _) = request(
        &app,
        Method::POST,
        "/api/auth/refresh",
        None,
        Some(json!({"refresh_token": refresh})),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, _) = request(
        &app,
        Method::POST,
        "/api/auth/logout",
        Some(&access),
        Some(json!({"refresh_token": rotated.clone()})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = request(&app, Method::GET, "/api/auth/me", Some(&access), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = request(
        &app,
        Method::POST,
        "/api/auth/refresh",
        None,
        Some(json!({"refresh_token": rotated})),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}
