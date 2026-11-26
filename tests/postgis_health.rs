mod common;

use axum::http::{Method, StatusCode};

use xingtu_tsp::{
    build_app,
    db::Db,
    models::{CreateVehicleRequest, LocationSample},
    state::AppState,
};

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
    let suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_millis()
        % 1_000_000;
    let vehicle = db
        .create_vehicle(CreateVehicleRequest {
            vin: format!("LSVAA2A41NX{suffix:06}"),
            plate_no: format!("PostGIS-{suffix:06}"),
            model: "PostGIS".to_string(),
            color: "white".to_string(),
            energy_type: "EV".to_string(),
            driver_name: "PostGIS".to_string(),
            driver_phone: "13600000000".to_string(),
            org_name: "PostGISOrg".to_string(),
        })
        .await
        .expect("vehicle");
    db.ingest_location(LocationSample {
        vehicle_id: vehicle.vehicle.id.clone(),
        ts: "2026-11-26T20:00:00+08:00".to_string(),
        lat: 31.2304,
        lon: 121.4737,
        speed: 10.0,
        heading: 90.0,
        ignition: true,
        soc: Some(80.0),
        odometer_km: Some(1.0),
        source: "postgis-test".to_string(),
        extra_json: None,
    })
    .await
    .expect("ingest");

    let state = AppState::new(config, db);
    let app = build_app(state.clone());

    let (status, body) = request(&app, Method::GET, "/api/health", None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["database_engine"], "postgres");
    assert_eq!(body["spatial_backend"], "postgis");

    let (status, ready) = request(&app, Method::GET, "/api/ready", None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(ready["spatial_backend"], "postgis");

    let bbox = state
        .db
        .vehicles_in_bbox(121.2, 31.0, 121.8, 31.4, None, "*")
        .await
        .expect("bbox");
    assert!(bbox.iter().any(|item| item.id == vehicle.vehicle.id));
    let nearby = state
        .db
        .vehicles_nearby(31.2304, 121.4737, 5000.0, None, "*")
        .await
        .expect("nearby");
    assert!(nearby.iter().any(|item| item.id == vehicle.vehicle.id));
}
