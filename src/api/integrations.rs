use axum::{extract::State, Json};
use serde_json::{json, Value};

use crate::{auth::AuthUser, error::AppResult, state::AppState};

/// Advertises the honest level of standards support implemented by this build.
/// Consumers must not treat `frame=true` as conformance.
pub async fn standards(State(_state): State<AppState>, _user: AuthUser) -> AppResult<Json<Value>> {
    let (done, partial, missing, na) = crate::standards::catalog::counts();
    Ok(Json(json!({
        "catalog_version": crate::standards::catalog::CATALOG_VERSION,
        "catalog_scope": "core-message-subset",
        "catalog_counts": {"done": done, "partial": partial, "missing": missing, "not_applicable": na},
        "jt_t_809_2019": {
            "envelope": true,
            "crc32": true,
            "business_messages": false,
            "encryption": false,
            "platform_handshake": false,
            "conformance_tested": false,
            "negative_codec_tests": true
        },
        "gb_t_32960_3_2025": {
            "envelope": true,
            "vin": true,
            "business_data_units": false,
            "encryption": false,
            "conformance_tested": false
        },
        "jt_t_808_2019": {
            "subset": true,
            "registration_auth_heartbeat_location": true,
            "registration_full_fields": false,
            "registration_dual_layout": true,
            "registration_plate_no": true,
            "attributes_candidate_fields": true,
            "batch_location": true,
            "subpackage_reassembly": true,
            "subpackage_limits": true,
            "text_downlink_0x8300": true,
            "vehicle_control_0x8500": true,
            "parameter_messages": true,
            "parameter_version_history": true,
            "parameter_version_diff": true,
            "parameter_retry": true,
            "downlink_timeout_detection": true,
            "parameter_auto_retry": true,
            "query_all_parameters_0x8104": true,
            "query_attributes_0x8107": true,
            "temp_tracking_0x8202": true,
            "temp_tracking_state": true,
            "link_test_state_0x8204": true,
            "alarm_ack_0x8203": true,
            "alarm_ack_candidate": true,
            "cross_batch_retransmission_dedupe": true,
            "concurrent_batch_idempotency_claims": true,
            "location_extra_items": true,
            "location_extra_semantics": true,
            "location_extra_strict_tlv": true,
            "location_range_validation": true,
            "alarm_id_extra": true,
            "overspeed_area_extra": true,
            "area_event_extra": true,
            "route_time_extra": true,
            "extended_signal_extra": true,
            "batch_chronological_sort": true,
            "single_session_local": true,
            "single_session_cross_node": true,
            "batch_limits": true,
            "batch_rate_limit": true,
            "parameter_response_persistence": true,
            "attributes_response_persistence": true,
            "encryption": false,
            "full_message_catalog": false,
            "conformance_tested": false
        }
    })))
}
