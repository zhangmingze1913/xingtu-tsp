mod common;

use axum::http::{Method, StatusCode};

use xingtu_tsp::{build_app, db::Db, state::AppState};

use common::{request, test_config};

/// Opt-in PostGIS smoke: set TSP_TEST_POSTGIS_URL to a disposable database.
/// This validates the PostgreSQL/PostGIS backend branch, not a real KGIS
/// license or Kingbase deployment.
#[tokio::test]
async fn health_reports_postgis_backend_when_configured() {
    let Ok(url) = std::env::var("TSP_TEST_POSTGIS_URL") else {
        return;
    };
    let mut config = test_config();
    config.database_url = url;
    config.postgis_enabled = true;
    let db = Db::open_with_postgis(&config.database_url, true)
        .await
        .expect("postgis db");
    let state = AppState::new(config, db);
    let app = build_app(state);

    let (status, body) = request(&app, Method::GET, "/api/health", None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["database_engine"], "postgres");
    assert_eq!(body["spatial_backend"], "postgis");

    let (status, ready) = request(&app, Method::GET, "/api/ready", None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(ready["spatial_backend"], "postgis");
}
