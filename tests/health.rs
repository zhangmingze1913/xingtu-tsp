mod common;

use axum::http::{Method, StatusCode};

use common::{request, setup};

#[tokio::test]
async fn health_reports_spatial_backend() {
    let (app, _state) = setup().await;
    let (status, body) = request(&app, Method::GET, "/api/health", None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["database_engine"], "sqlite");
    assert_eq!(body["spatial_backend"], "lat_lon_fallback");
    assert_eq!(body["database"], true);
}
