//! GB/T 32960 simulated conformance smoke suite.
//! Covers implemented commands/sections only; not certification evidence.

use xingtu_tsp::standards::gb32960::{
    self, Gb32960AlarmData, Gb32960BatteryTemperature, Gb32960BatteryVoltage, Gb32960ExtremeData,
    Gb32960Frame, Gb32960LocationData, Gb32960Login, Gb32960Logout, Gb32960Motor, Gb32960MotorData,
    Gb32960PlatformAck, Gb32960PlatformLogin, Gb32960RealtimeData, Gb32960Section,
    GB32960_HEARTBEAT, GB32960_LOGIN, GB32960_LOGOUT, GB32960_PLATFORM_LOGIN,
    GB32960_PLATFORM_LOGOUT, GB32960_REALTIME, GB32960_SUPPLEMENT, SEC_ALARM, SEC_BATTERY_TEMP,
    SEC_BATTERY_VOLTAGE, SEC_EXTREME, SEC_LOCATION, SEC_MOTOR, SEC_VEHICLE,
};

const VIN: &str = "LSVAA123456789012";

fn frame(command: u8, data: Vec<u8>) -> Gb32960Frame {
    Gb32960Frame {
        command,
        response: 0,
        vin: VIN.to_string(),
        encrypt: 0,
        data,
    }
}

#[test]
fn all_implemented_commands_roundtrip() {
    let login = Gb32960Login {
        login_time: [0x26, 0x11, 0x19, 0x10, 0x00, 0x00],
        serial: 1,
        iccid: "89860012345678901234".to_string(),
        subsystems: vec!["BAT001".to_string()],
    };
    let login_frame = frame(GB32960_LOGIN, login.encode().expect("login"));
    assert_eq!(
        Gb32960Login::decode(
            &Gb32960Frame::decode(&login_frame.encode().unwrap())
                .unwrap()
                .data
        )
        .unwrap(),
        login
    );

    let vehicle = Gb32960RealtimeData {
        vehicle_status: 1,
        charge_status: 2,
        run_mode: 3,
        speed_kmh: 50.0,
        odometer_km: 100.0,
        total_voltage_v: 350.0,
        total_current_a: -10.0,
        soc_percent: 80,
        dc_dc_status: 1,
        gear: 4,
        insulation_kohm: 500,
        accel_pedal_percent: 20,
        brake_pedal_percent: 0,
        tail: vec![],
    };
    let motor = Gb32960MotorData {
        motors: vec![Gb32960Motor {
            serial: 1,
            status: 1,
            controller_temp_raw: 40,
            speed_raw: 3000,
            torque_raw: 2000,
            temp_raw: 45,
        }],
    };
    let alarm = Gb32960AlarmData {
        level: 1,
        flags: 1,
        battery_faults: vec![1],
        motor_faults: vec![],
        engine_faults: vec![],
        other_faults: vec![],
        general_alarms: vec![],
    };
    let location = Gb32960LocationData {
        status: 1,
        lon: 121.4737,
        lat: 31.2304,
        altitude_m: 12,
    };
    let voltage = Gb32960BatteryVoltage {
        subsystem: 1,
        cell_mv: vec![3200, 3300],
    };
    let temperature = Gb32960BatteryTemperature {
        subsystem: 1,
        probes: vec![40, 41],
    };
    let extreme = Gb32960ExtremeData {
        max_voltage_subsystem: 1,
        max_voltage_cell: 1,
        max_voltage_raw: 3600,
        min_voltage_subsystem: 1,
        min_voltage_cell: 2,
        min_voltage_raw: 3100,
        max_temp_subsystem: 1,
        max_temp_probe: 1,
        max_temp_raw: 80,
        min_temp_subsystem: 1,
        min_temp_probe: 2,
        min_temp_raw: 45,
    };

    let sections = vec![
        Gb32960Section {
            data_type: SEC_VEHICLE,
            data: vehicle.encode(),
        },
        Gb32960Section {
            data_type: SEC_MOTOR,
            data: motor.encode().unwrap(),
        },
        Gb32960Section {
            data_type: SEC_ALARM,
            data: alarm.encode().unwrap(),
        },
        Gb32960Section {
            data_type: SEC_LOCATION,
            data: location.encode().unwrap(),
        },
        Gb32960Section {
            data_type: SEC_BATTERY_VOLTAGE,
            data: voltage.encode().unwrap(),
        },
        Gb32960Section {
            data_type: SEC_BATTERY_TEMP,
            data: temperature.encode().unwrap(),
        },
        Gb32960Section {
            data_type: SEC_EXTREME,
            data: extreme.encode(),
        },
    ];
    let mut body = Vec::new();
    for section in &sections {
        body.extend_from_slice(&section.encode());
    }

    for command in [GB32960_REALTIME, GB32960_SUPPLEMENT] {
        let encoded = frame(command, body.clone()).encode().unwrap();
        let decoded = Gb32960Frame::decode(&encoded).unwrap();
        let unit = decoded.parse_unit().unwrap();
        assert_eq!(unit.sections, sections);
        assert_eq!(unit.vehicle().unwrap(), vehicle);
    }

    let logout = Gb32960Logout {
        time: [0x26, 0x11, 0x19, 0x10, 0x00, 0x00],
        serial: 1,
    };
    let decoded =
        Gb32960Frame::decode(&frame(GB32960_LOGOUT, logout.encode()).encode().unwrap()).unwrap();
    assert_eq!(Gb32960Logout::decode(&decoded.data).unwrap(), logout);

    let platform_login = Gb32960PlatformLogin {
        login_time: [0x26, 0x11, 0x19, 0x10, 0x00, 0x00],
        serial: 1,
        username: "platform".to_string(),
        password: "secret".to_string(),
    };
    let decoded = Gb32960Frame::decode(
        &frame(GB32960_PLATFORM_LOGIN, platform_login.encode().unwrap())
            .encode()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        Gb32960PlatformLogin::decode(&decoded.data).unwrap(),
        platform_login
    );

    let decoded = Gb32960Frame::decode(
        &frame(GB32960_PLATFORM_LOGOUT, logout.encode())
            .encode()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(Gb32960Logout::decode(&decoded.data).unwrap(), logout);

    let heartbeat =
        Gb32960Frame::decode(&frame(GB32960_HEARTBEAT, vec![]).encode().unwrap()).unwrap();
    assert!(heartbeat.data.is_empty());

    let ack = Gb32960PlatformAck {
        time: [0x26, 0x11, 0x19, 0x10, 0x00, 0x00],
        serial: 1,
    };
    assert_eq!(Gb32960PlatformAck::decode(&ack.encode()).unwrap(), ack);

    // Struct-level smoke check only; does not assert regulator acceptance.
    let _ = gb32960::SEC_SIGNAL;
}

#[tokio::test]
async fn tcp_gateway_ingests_frame_and_persists_trajectory() {
    use tokio::io::AsyncWriteExt;
    use tokio::net::{TcpListener, TcpStream};
    use xingtu_tsp::db::Db;
    use xingtu_tsp::models::CreateVehicleRequest;
    use xingtu_tsp::standards::gb32960_gateway::serve_listener;

    let vin = "LSVAA32960TCP0001";
    let db = Db::in_memory().await.expect("db");
    db.create_vehicle(CreateVehicleRequest {
        vin: vin.to_string(),
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

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let handle = tokio::spawn(serve_listener(listener, db.clone()));

    let mut stream = TcpStream::connect(addr).await.expect("connect");
    let vehicle = Gb32960RealtimeData {
        vehicle_status: 1,
        charge_status: 1,
        run_mode: 1,
        speed_kmh: 30.0,
        odometer_km: 10.0,
        total_voltage_v: 350.0,
        total_current_a: 5.0,
        soc_percent: 60,
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
        altitude_m: 8,
    };
    let mut data = vec![0x26, 0x11, 0x23, 0x21, 0x56, 0x00];
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
        vin: vin.to_string(),
        encrypt: 1,
        data,
    }
    .encode()
    .expect("frame");
    stream.write_all(&raw).await.expect("write");
    stream.flush().await.expect("flush");

    let vehicle_id = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            if let Some(id) = db.find_vehicle_id_by_vin(vin).await.expect("lookup") {
                let points = db
                    .trajectory(&id, None, None, 10)
                    .await
                    .expect("trajectory");
                if !points.is_empty() {
                    break id;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("ingested");
    assert_eq!(vehicle_id.len(), 36);
    handle.abort();
}
