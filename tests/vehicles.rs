mod common;

use axum::{http::Method, http::StatusCode};
use serde_json::json;

use common::{login, request, request_text, setup, test_config};
use xingtu_tsp::{auth::hash_password, build_app, db::Db, seed, state::AppState};

#[tokio::test]
async fn fleet_ingest_geofence_and_command_flow() {
    let (app, _state) = setup().await;
    let token = login(&app).await;

    let (status, body) = request(
        &app,
        Method::POST,
        "/api/vehicles",
        Some(&token),
        Some(json!({
            "vin": "LSVAA2A41NX999999",
            "plate_no": "沪A·T9999",
            "model": "测试车型",
            "color": "测试白",
            "energy_type": "EV",
            "driver_name": "测试员",
            "driver_phone": "13800000000",
            "org_name": "集成测试车队"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "create failed: {body}");
    let vehicle_id = body["id"].as_str().expect("vehicle id").to_string();

    let (status, duplicate) = request(
        &app,
        Method::POST,
        "/api/vehicles",
        Some(&token),
        Some(json!({
            "vin": "LSVAA2A41NX999999",
            "plate_no": "沪A·T8888",
            "model": "重复车辆",
            "energy_type": "EV"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "duplicate vin: {duplicate}");
    assert_eq!(duplicate["error"]["code"], "conflict");

    let (status, body) = request(
        &app,
        Method::POST,
        "/api/telemetry",
        Some(&token),
        Some(json!({
            "vehicle_id": vehicle_id,
            "lat": 31.2304,
            "lon": 121.4737,
            "speed": 150.0,
            "heading": 90.0,
            "ignition": true,
            "soc": 62.0,
            "odometer_km": 1200.0,
            "source": "integration-test"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "ingest failed: {body}");
    assert_eq!(body["status"], "moving");
    assert!(body["trip_started"].as_bool().unwrap_or(false));
    let has_overspeed = body["alerts"]
        .as_array()
        .expect("alerts")
        .iter()
        .any(|alert| alert["alert_type"] == "overspeed");
    assert!(has_overspeed, "overspeed alert missing: {body}");

    let (status, fence) = request(
        &app,
        Method::POST,
        "/api/geofences",
        Some(&token),
        Some(json!({
            "name": "集成测试围栏",
            "kind": "circle",
            "center_lat": 31.2304,
            "center_lon": 121.4737,
            "radius_m": 500.0,
            "color": "#22d3ee",
            "active": true
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "fence failed: {fence}");

    let sample = |lat: f64, lon: f64, speed: f64| {
        json!({
            "vehicle_id": vehicle_id,
            "lat": lat,
            "lon": lon,
            "speed": speed,
            "ignition": true,
            "soc": 60.0,
            "source": "integration-test"
        })
    };
    let (status, _) = request(
        &app,
        Method::POST,
        "/api/telemetry",
        Some(&token),
        Some(sample(31.2404, 121.4737, 30.0)),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, enter) = request(
        &app,
        Method::POST,
        "/api/telemetry",
        Some(&token),
        Some(sample(31.2306, 121.4737, 25.0)),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "enter failed: {enter}");
    assert!(enter["alerts"]
        .as_array()
        .expect("alerts")
        .iter()
        .any(|alert| alert["alert_type"] == "geofence_enter"));
    let (status, exit) = request(
        &app,
        Method::POST,
        "/api/telemetry",
        Some(&token),
        Some(sample(31.2450, 121.4737, 28.0)),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert!(exit["alerts"]
        .as_array()
        .expect("alerts")
        .iter()
        .any(|alert| alert["alert_type"] == "geofence_exit"));

    let (status, trajectory) = request(
        &app,
        Method::GET,
        &format!("/api/vehicles/{vehicle_id}/trajectory?limit=100"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(trajectory.as_array().expect("trajectory").len(), 4);

    let (status, command) = request(
        &app,
        Method::POST,
        &format!("/api/vehicles/{vehicle_id}/commands"),
        Some(&token),
        Some(json!({"command": "lock"})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "command failed: {command}");
    assert_eq!(command["status"], "pending");

    let (status, dashboard) =
        request(&app, Method::GET, "/api/dashboard", Some(&token), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(dashboard["fleet"]["total"], 1);
    assert_eq!(dashboard["fleet"]["moving"], 1);
    assert!(dashboard["alerts"]["active"].as_i64().unwrap_or(0) >= 3);

    let (status, page) = request(
        &app,
        Method::GET,
        "/api/alerts?status=active&page_size=50",
        Some(&token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(page["items"].as_array().expect("alert items").len() >= 3);
}

#[tokio::test]
async fn viewer_alert_and_trip_plate_are_masked() {
    let db = Db::in_memory().await.expect("in-memory db");
    seed::seed_if_empty(&db).await.expect("seed");
    let state = AppState::new(test_config(), db);
    let app = build_app(state);
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

    let (status, alerts) = request(
        &app,
        Method::GET,
        "/api/alerts?page_size=50",
        Some(token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let alert_items = alerts["items"].as_array().expect("alerts");
    assert!(!alert_items.is_empty());
    for item in alert_items {
        if let Some(plate) = item["plate_no"].as_str() {
            assert!(plate.contains('*'));
        }
    }

    let (status, trips) = request(
        &app,
        Method::GET,
        "/api/trips?page_size=50",
        Some(token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let trip_items = trips["items"].as_array().expect("trips");
    assert!(!trip_items.is_empty());
    for item in trip_items {
        if let Some(plate) = item["plate_no"].as_str() {
            assert!(plate.contains('*'));
        }
    }

    let (status, csv) = request_text(&app, Method::GET, "/api/trips/export", Some(token)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(csv.contains("plate_no"));
    assert!(csv.contains('*'));
}

#[tokio::test]
async fn viewer_terminal_pii_is_masked() {
    let db = Db::in_memory().await.expect("in-memory db");
    seed::seed_if_empty(&db).await.expect("seed");
    let state = AppState::new(test_config(), db);
    let app = build_app(state);
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
    let (status, rows) = request(&app, Method::GET, "/api/terminals", Some(token), None).await;
    assert_eq!(status, StatusCode::OK);
    let items = rows.as_array().expect("terminals");
    assert!(!items.is_empty());
    for item in items {
        assert!(item["phone"].as_str().unwrap().contains('*'));
        assert!(item["iccid"].as_str().unwrap().contains('*'));
        assert!(item["imei"].as_str().unwrap().contains('*'));
    }

    let (status, csv) = request_text(&app, Method::GET, "/api/terminals/export", Some(token)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(csv.contains("imei,phone,iccid"));
    assert!(csv.contains('*'));
}

#[tokio::test]
async fn media_endpoints_require_authentication() {
    let (app, state) = setup().await;
    let hash = hash_password("viewer123").expect("hash");
    state
        .db
        .create_user("viewer", &hash, "Viewer", "viewer", "")
        .await
        .expect("viewer");
    let (status, _) = request(&app, Method::GET, "/api/media/sessions", None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let token = login(&app).await;
    let (status, body) =
        request(&app, Method::GET, "/api/media/sessions", Some(&token), None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body["sessions"].is_array());

    let (status, body) = request(
        &app,
        Method::POST,
        "/api/auth/login",
        None,
        Some(json!({"username": "viewer", "password": "viewer123"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let viewer = body["token"].as_str().expect("viewer token");
    let (status, _) = request(&app, Method::GET, "/api/media/sessions", Some(viewer), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn raw_808_command_requires_admin() {
    let (app, state) = setup().await;
    let hash = hash_password("operator123").expect("hash");
    state
        .db
        .create_user("operator", &hash, "Operator", "operator", "")
        .await
        .expect("operator");
    let (status, body) = request(
        &app,
        Method::POST,
        "/api/auth/login",
        None,
        Some(json!({"username": "operator", "password": "operator123"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let token = body["token"].as_str().expect("token");
    let (status, _) = request(
        &app,
        Method::POST,
        "/api/vehicles/missing/commands",
        Some(token),
        Some(json!({"command": "raw_808", "payload": "0x8600:0102"})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn readiness_reports_database_and_cluster_state() {
    let (app, _state) = setup().await;
    let (status, body) = request(&app, Method::GET, "/api/ready", None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["ready"], true);
    assert_eq!(body["database"], true);
    assert_eq!(body["cluster_enabled"], false);
}

#[tokio::test]
async fn compliance_report_is_admin_only_and_honest() {
    let (app, _state) = setup().await;
    let token = login(&app).await;
    let (status, body) = request(
        &app,
        Method::GET,
        "/api/security/compliance",
        Some(&token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["scope"], "code-side evidence; not a certification");
    let controls = body["controls"].as_array().expect("controls");
    assert!(controls.iter().any(|control| control["id"] == "CRYPTO-01"));
    assert!(controls
        .iter()
        .any(|control| control["status"] == "external"));
    assert!(!controls
        .iter()
        .any(|control| control["status"] == "certified"));
}

#[tokio::test]
async fn viewer_vehicle_csv_export_is_masked() {
    let db = Db::in_memory().await.expect("in-memory db");
    seed::seed_if_empty(&db).await.expect("seed");
    let state = AppState::new(test_config(), db);
    let app = build_app(state);
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
    let (status, csv) = request_text(&app, Method::GET, "/api/vehicles/export", Some(token)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(csv.contains("vin,plate_no"));
    assert!(csv.contains('*'));
    assert!(!csv.contains("LSVAA"));

    let (status, page) = request(
        &app,
        Method::GET,
        "/api/vehicles?page_size=1",
        Some(token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let id = page["items"][0]["id"].as_str().expect("vehicle id");
    let (status, trajectory) = request_text(
        &app,
        Method::GET,
        &format!("/api/vehicles/{id}/trajectory/export"),
        Some(token),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(trajectory.contains("ts,lat,lon,speed"));
}

#[tokio::test]
async fn viewer_vehicle_pii_is_masked() {
    let db = Db::in_memory().await.expect("in-memory db");
    seed::seed_if_empty(&db).await.expect("seed");
    let state = AppState::new(test_config(), db);
    let app = build_app(state);
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
    let (status, page) = request(
        &app,
        Method::GET,
        "/api/vehicles?page_size=50",
        Some(token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let items = page["items"].as_array().expect("items");
    assert!(!items.is_empty());
    for item in items {
        assert!(item["vin"].as_str().unwrap().contains('*'));
        assert!(item["plate_no"].as_str().unwrap().contains('*'));
        assert!(item["driver_name"].as_str().unwrap().contains('*'));
        assert!(item["driver_phone"].as_str().unwrap().contains('*'));
    }
}

#[tokio::test]
async fn operator_vehicle_scope_is_org_limited() {
    let db = Db::in_memory().await.expect("in-memory db");
    seed::seed_if_empty(&db).await.expect("seed");
    let state = AppState::new(test_config(), db);
    let app = build_app(state);
    let (status, body) = request(
        &app,
        Method::POST,
        "/api/auth/login",
        None,
        Some(json!({"username": "operator", "password": "operator123"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let token = body["token"].as_str().expect("token");
    let (status, page) = request(
        &app,
        Method::GET,
        "/api/vehicles?page_size=50",
        Some(token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let items = page["items"].as_array().expect("items");
    assert!(!items.is_empty());
    assert!(
        items
            .iter()
            .all(|item| item["org_name"] == "星途出行 · 浦东车队"),
        "operator must only see its own organization"
    );
    assert!(page["total"].as_i64().unwrap_or(0) < 18);
    let (status, dashboard) = request(&app, Method::GET, "/api/dashboard", Some(token), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(dashboard["fleet"]["total"], 6);
    let (status, fence) = request(
        &app,
        Method::POST,
        "/api/geofences",
        Some(token),
        Some(json!({
            "name": "operator-fence",
            "kind": "circle",
            "center_lat": 31.2304,
            "center_lon": 121.4737,
            "radius_m": 300.0,
            "active": true
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(fence["org_name"], "星途出行 · 浦东车队");
}

#[tokio::test]
async fn update_and_acknowledge_paths_work() {
    let (app, _state) = setup().await;
    let token = login(&app).await;

    let (status, body) = request(
        &app,
        Method::POST,
        "/api/vehicles",
        Some(&token),
        Some(json!({
            "vin": "LSVAA2A41NX777777",
            "plate_no": "沪A·T7777",
            "model": "更新测试",
            "color": "初始白",
            "energy_type": "EV",
            "driver_name": "原司机"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "create failed: {body}");
    let vehicle_id = body["id"].as_str().expect("vehicle id").to_string();

    // This UPDATE starts at ?2 and ends at ?1, so it only works if numbered
    // placeholders keep their SQLite/PostgreSQL semantics.
    let (status, updated) = request(
        &app,
        Method::PATCH,
        &format!("/api/vehicles/{vehicle_id}"),
        Some(&token),
        Some(json!({"color": "星河蓝", "driver_name": "新司机"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "update failed: {updated}");
    assert_eq!(updated["color"], "星河蓝");
    assert_eq!(updated["driver_name"], "新司机");

    let (status, fence) = request(
        &app,
        Method::POST,
        "/api/geofences",
        Some(&token),
        Some(json!({
            "name": "待改名围栏",
            "kind": "circle",
            "center_lat": 31.2304,
            "center_lon": 121.4737,
            "radius_m": 500.0
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "fence failed: {fence}");
    let fence_id = fence["id"].as_str().expect("fence id").to_string();

    let (status, fence) = request(
        &app,
        Method::PATCH,
        &format!("/api/geofences/{fence_id}"),
        Some(&token),
        Some(json!({"name": "改名围栏", "radius_m": 900.0, "active": false})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "fence update failed: {fence}");
    assert_eq!(fence["name"], "改名围栏");
    assert_eq!(fence["radius_m"], 900.0);
    assert_eq!(fence["active"], false);

    let (status, outcome) = request(
        &app,
        Method::POST,
        "/api/telemetry",
        Some(&token),
        Some(json!({
            "vehicle_id": vehicle_id,
            "lat": 31.2304,
            "lon": 121.4737,
            "speed": 150.0,
            "heading": 90.0,
            "ignition": true,
            "soc": 55.0,
            "source": "integration-test"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "ingest failed: {outcome}");
    let alert_id = outcome["alerts"]
        .as_array()
        .expect("alerts")
        .iter()
        .find(|alert| alert["alert_type"] == "overspeed")
        .and_then(|alert| alert["id"].as_str())
        .expect("overspeed alert")
        .to_string();

    // ACK uses ?2/?3 before ?1 in its UPDATE statement.
    let (status, acked) = request(
        &app,
        Method::POST,
        &format!("/api/alerts/{alert_id}/ack"),
        Some(&token),
        Some(json!({"note": "测试确认"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "ack failed: {acked}");
    assert_eq!(acked["acknowledged"], true);
    assert_eq!(acked["ack_by"], "admin");
}

#[tokio::test]
async fn spatial_bbox_and_nearby_endpoints_work_with_fallback() {
    let db = Db::in_memory().await.expect("in-memory db");
    seed::seed_if_empty(&db).await.expect("seed");
    let state = AppState::new(test_config(), db);
    let app = build_app(state);
    let token = login(&app).await;
    let (status, bbox) = request(
        &app,
        Method::GET,
        "/api/vehicles/map?bbox=121.2,31.0,121.8,31.4",
        Some(&token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(!bbox.as_array().expect("bbox array").is_empty());
    let (status, nearby) = request(
        &app,
        Method::GET,
        "/api/vehicles/nearby?lat=31.23&lon=121.47&radius_m=5000",
        Some(&token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(!nearby.as_array().expect("nearby array").is_empty());
}

#[tokio::test]
async fn gb32960_device_registry_admin_flow() {
    let (app, _state) = setup().await;
    let token = login(&app).await;
    let vin = "LSVAA329600000077";
    let (status, body) = request(
        &app,
        Method::PUT,
        &format!("/api/gb32960/devices/{vin}"),
        Some(&token),
        Some(json!({ "enabled": true, "note": "fleet-a" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "enable failed: {body}");
    assert_eq!(body["enabled"], true);

    let (status, body) = request(
        &app,
        Method::GET,
        "/api/gb32960/devices",
        Some(&token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["vin"] == vin));

    let (status, body) = request(
        &app,
        Method::PUT,
        &format!("/api/gb32960/devices/{vin}"),
        Some(&token),
        Some(json!({ "enabled": false, "note": "revoked" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "disable failed: {body}");
    assert_eq!(body["enabled"], false);
}

#[tokio::test]
async fn pii_deletion_endpoint_clears_driver_fields() {
    let (app, _state) = setup().await;
    let token = login(&app).await;
    let (status, body) = request(
        &app,
        Method::POST,
        "/api/vehicles",
        Some(&token),
        Some(json!({
            "vin": "LSVAA2A41NX999998",
            "plate_no": "娌狝路PII998",
            "model": "PII",
            "color": "white",
            "energy_type": "EV",
            "driver_name": "PII Driver",
            "driver_phone": "13600000000",
            "org_name": "PIIOrg"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "create: {body}");
    let id = body["id"].as_str().expect("id");
    let (status, body) = request(
        &app,
        Method::DELETE,
        &format!("/api/vehicles/{id}/pii"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "erase: {body}");
    assert_eq!(body["deleted"], 0);
    let (status, body) = request(
        &app,
        Method::GET,
        &format!("/api/vehicles/{id}"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["driver_name"], "");
    assert_eq!(body["driver_phone"], "");
}
