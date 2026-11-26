mod common;

use axum::{http::Method, http::StatusCode};
use serde_json::json;

use common::{request, setup};

#[tokio::test]
async fn standards_capabilities_are_honest() {
    let (app, _state) = setup().await;
    let (status, body) = request(
        &app,
        Method::POST,
        "/api/auth/login",
        None,
        Some(json!({"username": "admin", "password": "admin123"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let token = body["token"].as_str().expect("token");
    let (status, body) = request(
        &app,
        Method::GET,
        "/api/integrations/standards",
        Some(token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["jt_t_809_2019"]["envelope"], true);
    assert_eq!(body["jt_t_809_2019"]["business_messages"], true);
    assert_eq!(body["gb_t_32960_3_2025"]["business_data_units"], true);
    assert_eq!(body["jt_t_808_2019"]["registration_full_fields"], true);
    assert!(body["catalog_categories"]["local"].as_u64().unwrap_or(0) > 0);
    assert!(body["catalog_categories"]["group"].as_u64().unwrap_or(0) > 0);
    assert!(body["standards"]
        .as_array()
        .map(|items| !items.is_empty())
        .unwrap_or(false));
    assert_eq!(body["jt_t_808_2019"]["query_all_parameters_0x8104"], true);
    assert_eq!(body["jt_t_808_2019"]["query_attributes_0x8107"], true);
    assert_eq!(body["jt_t_808_2019"]["text_downlink_0x8300"], true);
    assert_eq!(body["jt_t_808_2019"]["conformance_tested"], false);

    let (status, local) = request(
        &app,
        Method::GET,
        "/api/integrations/standards?category=local",
        Some(token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(local["filtered_count"].as_u64().unwrap_or(0) > 0);
    assert!(local["standards"]
        .as_array()
        .expect("local standards")
        .iter()
        .all(|spec| spec["category"] == "local"));

    let (status, invalid) = request(
        &app,
        Method::GET,
        "/api/integrations/standards?category=bogus",
        Some(token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{invalid}");
}
