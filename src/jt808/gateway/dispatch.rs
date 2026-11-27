//! JT/T 808 message dispatch for an authenticated connection.

use super::*;
use super::{downlink, registration, terminal_profile};

#[allow(clippy::too_many_arguments)]
pub(super) async fn process_frame(
    state: &AppState,
    frame: &Frame,
    serial: &AtomicU16,
    authenticated: &mut bool,
    peer: &str,
    sessions: &mut HashSet<String>,
    out_tx: &mpsc::UnboundedSender<Vec<u8>>,
    cancel: &Arc<AtomicBool>,
) -> Option<Vec<u8>> {
    let server_serial = next_serial(serial);
    match frame.msg_id {
        MSG_TERMINAL_REGISTER => registration::register(state, frame, server_serial).await,
        MSG_TERMINAL_AUTH => {
            registration::auth(
                state,
                frame,
                server_serial,
                authenticated,
                peer,
                sessions,
                out_tx,
                cancel,
            )
            .await
        }
        MSG_TERMINAL_RESPONSE => downlink::ack(state, frame, server_serial, *authenticated).await,
        MSG_QUERY_PARAMETERS_RESPONSE => {
            let result = if *authenticated {
                terminal_profile::parameters(state, frame).await
            } else {
                1
            };
            general_response(frame, result, server_serial).ok()
        }
        MSG_QUERY_ATTRIBUTES_RESPONSE => {
            let result = if *authenticated {
                terminal_profile::attributes(state, frame).await
            } else {
                1
            };
            general_response(frame, result, server_serial).ok()
        }
        MSG_LOCATION_QUERY => {
            if !*authenticated {
                return general_response(frame, 1, server_serial).ok();
            }
            let Some(terminal) = state
                .db
                .get_terminal_by_phone(&frame.terminal_id)
                .await
                .ok()
                .flatten()
            else {
                return general_response(frame, 1, server_serial).ok();
            };
            let Some(vehicle_id) = terminal.vehicle_id else {
                return general_response(frame, 1, server_serial).ok();
            };
            let Some(vehicle) = state.db.get_vehicle(&vehicle_id).await.ok().flatten() else {
                return general_response(frame, 1, server_serial).ok();
            };
            let (Some(lat), Some(lon)) = (vehicle.state.lat, vehicle.state.lon) else {
                return general_response(frame, 1, server_serial).ok();
            };
            let body = encode_location_response(
                lat,
                lon,
                vehicle.state.speed,
                vehicle.state.heading,
                vehicle.state.ignition,
                &vehicle.state.updated_at,
            );
            build_frame(
                MSG_LOCATION_QUERY_RESPONSE,
                &frame.terminal_id,
                server_serial,
                &body,
            )
            .ok()
        }
        MSG_LINK_TEST | MSG_TEMP_TRACKING | MSG_ALARM_ACK => {
            general_response(frame, if *authenticated { 0 } else { 1 }, server_serial).ok()
        }
        MSG_HEARTBEAT => {
            registration::heartbeat(
                state,
                frame,
                server_serial,
                *authenticated,
                peer,
                sessions,
                out_tx,
            )
            .await
        }
        MSG_LOCATION_REPORT => {
            let location = match parse_location(&frame.body) {
                Ok(location) => location,
                Err(error) => {
                    tracing::debug!(%error, terminal = %frame.terminal_id, "invalid JT808 location");
                    return general_response(frame, 1, server_serial).ok();
                }
            };
            let result = ingest_location(
                state,
                frame,
                &location,
                *authenticated,
                peer,
                sessions,
                out_tx,
            )
            .await;
            general_response(frame, result, server_serial).ok()
        }
        MSG_LOCATION_BATCH => {
            if !*authenticated {
                return general_response(frame, 1, server_serial).ok();
            }
            if !allow_location_batch(&frame.terminal_id) {
                state
                    .metrics
                    .telemetry_duplicates
                    .fetch_add(1, Ordering::Relaxed);
                tracing::warn!(terminal = %frame.terminal_id, "JT808 batch rate limited");
                return general_response(frame, 2, server_serial).ok();
            }
            let locations = match parse_location_batch(&frame.body) {
                Ok(locations) => locations,
                Err(error) => {
                    tracing::debug!(%error, terminal = %frame.terminal_id, "invalid JT808 batch");
                    return general_response(frame, 1, server_serial).ok();
                }
            };
            let result = ingest_batch(
                state,
                frame,
                &locations,
                *authenticated,
                peer,
                sessions,
                out_tx,
            )
            .await;
            general_response(frame, result, server_serial).ok()
        }
        _ => {
            // `format!` in a tracing field is evaluated eagerly, even when the
            // DEBUG level is disabled. Gate it so the hot path stays allocation
            // free for unsupported message ids.
            if tracing::enabled!(tracing::Level::DEBUG) {
                tracing::debug!(
                    msg_id = format!("{:#06x}", frame.msg_id),
                    "unsupported JT808 message"
                );
            }
            general_response(frame, 3, server_serial).ok()
        }
    }
}
