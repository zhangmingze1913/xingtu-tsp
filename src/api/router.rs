use axum::{
    http::{header, HeaderValue, Method},
    routing::{get, post},
    Router,
};
use tower_http::{
    compression::CompressionLayer,
    cors::CorsLayer,
    services::{ServeDir, ServeFile},
    trace::TraceLayer,
};

use super::{
    alerts, audit, auth, cluster, commands, dashboard, gb32960, geofences, health, integrations,
    media, security, telemetry, terminal_downlink, terminals, trips, vehicles, ws,
};
use crate::{error::AppError, state::AppState};

pub fn build_router(state: AppState) -> Router {
    let api = Router::new()
        .route("/health", get(health::health))
        .route("/ready", get(health::ready))
        .route("/metrics", get(cluster::metrics))
        .route("/cluster/status", get(cluster::status))
        .route("/auth/login", post(auth::login))
        .route("/auth/refresh", post(auth::refresh))
        .route("/auth/logout", post(auth::logout))
        .route("/auth/me", get(auth::me))
        .route("/dashboard", get(dashboard::overview))
        .route("/vehicles", get(vehicles::list).post(vehicles::create))
        .route("/vehicles/export", get(vehicles::export))
        .route("/vehicles/map", get(vehicles::map))
        .route("/vehicles/nearby", get(vehicles::nearby))
        .route(
            "/vehicles/{id}",
            get(vehicles::get)
                .patch(vehicles::update)
                .delete(vehicles::delete),
        )
        .route("/vehicles/{id}/trajectory", get(telemetry::trajectory))
        .route(
            "/vehicles/{id}/trajectory/export",
            get(telemetry::trajectory_export),
        )
        .route("/vehicles/{id}/trips", get(vehicles::trips))
        .route(
            "/vehicles/{id}/commands",
            get(commands::list_commands).post(commands::create_command),
        )
        .route("/vehicles/{id}/terminal", post(vehicles::bind_terminal))
        .route("/telemetry", post(telemetry::ingest))
        .route("/gb32960/ingest", post(gb32960::ingest_frame))
        .route("/gb32960/devices", get(gb32960::list_devices))
        .route(
            "/gb32960/devices/{vin}",
            axum::routing::put(gb32960::set_device),
        )
        .route("/trips", get(trips::list))
        .route("/trips/export", get(trips::export))
        .route("/trips/{id}", get(trips::get))
        .route("/geofences", get(geofences::list).post(geofences::create))
        .route(
            "/geofences/{id}",
            get(geofences::get)
                .patch(geofences::update)
                .delete(geofences::delete),
        )
        .route("/alerts", get(alerts::list))
        .route("/alerts/export", get(alerts::export))
        .route("/alerts/ack-all", post(alerts::ack_all))
        .route("/alerts/{id}/ack", post(alerts::ack))
        .route("/commands", get(commands::list))
        .route("/commands/{id}", get(commands::get))
        .route("/audit", get(audit::list))
        .route("/security/compliance", get(security::compliance))
        .route("/media/sample.mp4", get(media::sample_mp4))
        .route("/media/live/{sim}/{channel}", get(media::live_mp4))
        .route(
            "/media/sessions/{sim}/{channel}",
            axum::routing::delete(media::close_session),
        )
        .route(
            "/media/live/{sim}/{channel}/stream",
            get(media::live_stream),
        )
        .route("/media/sessions", get(media::sessions))
        .route("/media/archive", get(media::archive_list))
        .route("/media/archive/playback", get(media::archive_playback))
        .route("/media/archive/{*key}", get(media::archive_get))
        .route("/audit/export", get(audit::export))
        .route("/integrations/standards", get(integrations::standards))
        .route(
            "/integrations/timeseries/reconcile",
            get(integrations::timeseries_reconcile),
        )
        .route("/terminals", get(terminals::list))
        .route("/terminals/export", get(terminals::export))
        .route("/terminals/{id}", get(terminals::get))
        .route("/terminals/{id}/bind", post(terminals::bind))
        .route(
            "/terminals/{phone}/parameters",
            get(terminal_downlink::get_parameters).post(terminal_downlink::set_parameters),
        )
        .route(
            "/terminals/{phone}/attributes",
            get(terminal_downlink::get_attributes).post(terminal_downlink::query_attributes),
        )
        .route(
            "/terminals/{phone}/parameters/history",
            get(terminal_downlink::parameter_history),
        )
        .route(
            "/terminals/{phone}/parameters/diff",
            get(terminal_downlink::parameter_diff),
        )
        .route(
            "/terminals/{phone}/parameters/retry",
            post(terminal_downlink::retry_parameters),
        )
        .route(
            "/terminals/{phone}/control",
            post(terminal_downlink::control),
        )
        .route(
            "/terminals/{phone}/vehicle-control",
            post(terminal_downlink::vehicle_control),
        )
        .route(
            "/terminals/{phone}/downlinks/{serial}",
            get(terminal_downlink::downlink_status),
        )
        .route(
            "/terminals/{phone}/query-location",
            post(terminal_downlink::query_location),
        )
        .route(
            "/terminals/{phone}/query-parameters",
            post(terminal_downlink::query_parameters),
        )
        .route(
            "/terminals/{phone}/query-all-parameters",
            post(terminal_downlink::query_all_parameters),
        )
        .route(
            "/terminals/{phone}/text",
            post(terminal_downlink::text_command),
        )
        .route(
            "/terminals/{phone}/temp-tracking",
            get(terminal_downlink::get_temp_tracking).post(terminal_downlink::temp_tracking),
        )
        .route(
            "/terminals/{phone}/link-test",
            get(terminal_downlink::get_link_test).post(terminal_downlink::link_test),
        )
        .route(
            "/terminals/{phone}/alarm-ack",
            post(terminal_downlink::alarm_ack),
        )
        .route("/ws", get(ws::ws_handler))
        .fallback(api_not_found)
        .with_state(state.clone());

    let index = state.config.web_dir.join("index.html");
    let static_site = ServeDir::new(&state.config.web_dir).fallback(ServeFile::new(index));

    let mut cors = CorsLayer::new()
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PATCH,
            Method::DELETE,
            Method::OPTIONS,
        ])
        .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE]);
    if !state.config.cors_origins.is_empty() {
        let origins: Vec<HeaderValue> = state
            .config
            .cors_origins
            .iter()
            .filter_map(|origin| HeaderValue::from_str(origin).ok())
            .collect();
        cors = cors.allow_origin(origins);
    }

    let jwks = Router::new()
        .route("/.well-known/jwks.json", get(auth::jwks))
        .route("/media", get(media::player))
        .with_state(state.clone());

    Router::new()
        .merge(jwks)
        .nest("/api", api)
        .fallback_service(static_site)
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::metrics::count_http,
        ))
        .layer(CompressionLayer::new())
        .layer(cors)
        .layer(TraceLayer::new_for_http())
}

async fn api_not_found() -> AppError {
    AppError::not_found("接口不存在")
}
