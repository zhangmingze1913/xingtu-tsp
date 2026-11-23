//! GB/T 32960.3 receive → decode → VIN lookup → telemetry ingest.

use chrono::{FixedOffset, NaiveDateTime, SecondsFormat, TimeZone};
use serde::Serialize;

use crate::{
    db::Db,
    error::{AppError, AppResult},
    models::LocationSample,
};

use super::gb32960::{
    decrypt_aes_128, Gb32960AesScheme, Gb32960Frame, Gb32960LocationData, GB32960_REALTIME,
    GB32960_SUPPLEMENT, SEC_LOCATION,
};

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Gb32960IngestResult {
    pub vin: String,
    pub vehicle_id: String,
    pub ts: String,
    pub inserted: bool,
}

/// Decode one plaintext 32960 realtime/supplement frame and persist its
/// location sample. Encrypted frames are rejected until a key provider is
/// bound by the caller.
pub async fn ingest(db: &Db, raw: &[u8]) -> AppResult<Gb32960IngestResult> {
    let frame = Gb32960Frame::decode(raw)
        .map_err(|error| AppError::bad_request(format!("GB/T 32960 帧错误: {error}")))?;
    persist(db, frame).await
}

/// Decrypt with a server-held AES-128 key and the caller-selected mode.
/// The 2025 text used here does not fix the mode/IV, so the caller must choose
/// explicitly rather than deriving it from the frame's encryption byte.
pub async fn ingest_encrypted(
    db: &Db,
    raw: &[u8],
    key: &[u8; 16],
    nonce: &[u8; 12],
    scheme: Gb32960AesScheme,
) -> AppResult<Gb32960IngestResult> {
    let frame = Gb32960Frame::decode_encrypted(raw, |_method, data| {
        decrypt_aes_128(key, nonce, data, scheme)
    })
    .map_err(|error| AppError::bad_request(format!("GB/T 32960 密文帧错误: {error}")))?;
    persist(db, frame).await
}

async fn persist(db: &Db, frame: Gb32960Frame) -> AppResult<Gb32960IngestResult> {
    if frame.command != GB32960_REALTIME && frame.command != GB32960_SUPPLEMENT {
        return Err(AppError::bad_request("仅支持 0x02/0x03 实时或补发帧"));
    }
    let (time, unit) = frame
        .parse_unit_with_time()
        .map_err(|error| AppError::bad_request(format!("GB/T 32960 数据单元错误: {error}")))?;
    let vehicle = unit
        .vehicle()
        .map_err(|error| AppError::bad_request(format!("整车数据错误: {error}")))?;
    let location_section = unit
        .section(SEC_LOCATION)
        .ok_or_else(|| AppError::bad_request("缺少车辆位置数据"))?;
    let location = Gb32960LocationData::decode(&location_section.data)
        .map_err(|error| AppError::bad_request(format!("位置数据错误: {error}")))?;
    let ts = bcd_time_to_iso(time)?;
    let vehicle_id = db
        .find_vehicle_id_by_vin(&frame.vin)
        .await?
        .ok_or_else(|| AppError::not_found("VIN 未注册"))?;
    let sample = LocationSample {
        vehicle_id: vehicle_id.clone(),
        ts: ts.clone(),
        lat: location.lat,
        lon: location.lon,
        speed: vehicle.speed_kmh,
        heading: 0.0,
        ignition: vehicle.vehicle_status == 1,
        soc: Some(vehicle.soc_percent as f64),
        odometer_km: Some(vehicle.odometer_km),
        source: "gb32960".to_string(),
        extra_json: None,
    };
    let inserted = db.ingest_location(sample).await?.is_some();
    Ok(Gb32960IngestResult {
        vin: frame.vin,
        vehicle_id,
        ts,
        inserted,
    })
}

fn bcd_time_to_iso(time: [u8; 6]) -> AppResult<String> {
    let digits: String = time
        .iter()
        .flat_map(|byte| {
            let high = byte >> 4;
            let low = byte & 0x0F;
            if high > 9 || low > 9 {
                return vec!['?', '?'];
            }
            vec![(b'0' + high) as char, (b'0' + low) as char]
        })
        .collect();
    let text = format!("20{digits}");
    let naive = NaiveDateTime::parse_from_str(&text, "%Y%m%d%H%M%S")
        .map_err(|_| AppError::bad_request("32960 时间 BCD 非法"))?;
    let offset = FixedOffset::east_opt(8 * 3600).ok_or_else(|| AppError::internal("时区错误"))?;
    let local = offset
        .from_local_datetime(&naive)
        .single()
        .ok_or_else(|| AppError::bad_request("32960 时间非法"))?;
    Ok(local.to_utc().to_rfc3339_opts(SecondsFormat::Secs, true))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::CreateVehicleRequest;
    use crate::standards::gb32960::{
        Gb32960Frame, Gb32960RealtimeData, Gb32960Section, GB32960_REALTIME, SEC_VEHICLE,
    };

    #[tokio::test]
    async fn realtime_frame_is_persisted_by_vin() {
        let db = Db::in_memory().await.expect("db");
        db.create_vehicle(CreateVehicleRequest {
            vin: "LSVAA329600000001".to_string(),
            plate_no: "沪A32960".to_string(),
            model: "EV".to_string(),
            color: "white".to_string(),
            energy_type: "EV".to_string(),
            driver_name: "Driver".to_string(),
            driver_phone: "13800138000".to_string(),
            org_name: "EVOrg".to_string(),
        })
        .await
        .expect("vehicle");

        let vehicle = Gb32960RealtimeData {
            vehicle_status: 1,
            charge_status: 1,
            run_mode: 1,
            speed_kmh: 42.5,
            odometer_km: 100.0,
            total_voltage_v: 350.0,
            total_current_a: 10.0,
            soc_percent: 80,
            dc_dc_status: 1,
            gear: 1,
            insulation_kohm: 500,
            accel_pedal_percent: 10,
            brake_pedal_percent: 0,
            tail: vec![],
        };
        let location = Gb32960LocationData {
            status: 1,
            lon: 121.4737,
            lat: 31.2304,
            altitude_m: 10,
        };
        let mut data = vec![0x26, 0x11, 0x23, 0x10, 0x30, 0x00];
        data.extend_from_slice(
            &Gb32960Section {
                data_type: SEC_VEHICLE,
                data: vehicle.encode(),
            }
            .encode(),
        );
        data.extend_from_slice(
            &Gb32960Section {
                data_type: SEC_LOCATION,
                data: location.encode().unwrap(),
            }
            .encode(),
        );
        let raw = Gb32960Frame {
            command: GB32960_REALTIME,
            response: 0xFE,
            vin: "LSVAA329600000001".to_string(),
            encrypt: 1,
            data,
        }
        .encode()
        .expect("frame");

        let result = ingest(&db, &raw).await.expect("ingest");
        assert_eq!(result.vin, "LSVAA329600000001");
        assert!(result.inserted);
        assert!(result.ts.ends_with('Z'));
    }

    #[tokio::test]
    async fn encrypted_frame_is_persisted() {
        use crate::standards::gb32960::{encrypt_aes_128, Gb32960AesScheme};
        let db = Db::in_memory().await.expect("db");
        db.create_vehicle(CreateVehicleRequest {
            vin: "LSVAA329600000002".to_string(),
            plate_no: "沪A32961".to_string(),
            model: "EV".to_string(),
            color: "white".to_string(),
            energy_type: "EV".to_string(),
            driver_name: "Driver".to_string(),
            driver_phone: "13800138001".to_string(),
            org_name: "EVOrg".to_string(),
        })
        .await
        .expect("vehicle");
        let vehicle = Gb32960RealtimeData {
            vehicle_status: 1,
            charge_status: 1,
            run_mode: 1,
            speed_kmh: 30.0,
            odometer_km: 5.0,
            total_voltage_v: 350.0,
            total_current_a: 5.0,
            soc_percent: 70,
            dc_dc_status: 1,
            gear: 1,
            insulation_kohm: 500,
            accel_pedal_percent: 5,
            brake_pedal_percent: 0,
            tail: vec![],
        };
        let location = Gb32960LocationData {
            status: 1,
            lon: 121.0,
            lat: 31.0,
            altitude_m: 5,
        };
        let mut plain = vec![0x26, 0x11, 0x23, 0x11, 0x00, 0x00];
        plain.extend_from_slice(
            &Gb32960Section {
                data_type: SEC_VEHICLE,
                data: vehicle.encode(),
            }
            .encode(),
        );
        plain.extend_from_slice(
            &Gb32960Section {
                data_type: SEC_LOCATION,
                data: location.encode().unwrap(),
            }
            .encode(),
        );
        let key = [7u8; 16];
        let nonce = [9u8; 12];
        let cipher = encrypt_aes_128(&key, &nonce, &plain, Gb32960AesScheme::Gcm).unwrap();
        let raw = Gb32960Frame {
            command: GB32960_REALTIME,
            response: 0xFE,
            vin: "LSVAA329600000002".to_string(),
            encrypt: 3,
            data: cipher,
        }
        .encode()
        .unwrap();
        let result = ingest_encrypted(&db, &raw, &key, &nonce, Gb32960AesScheme::Gcm)
            .await
            .expect("ingest");
        assert!(result.inserted);
        assert_eq!(result.vin, "LSVAA329600000002");
        assert!(
            ingest_encrypted(&db, &raw, &[0u8; 16], &nonce, Gb32960AesScheme::Gcm)
                .await
                .is_err()
        );
    }
}
