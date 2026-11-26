//! Integration: terminal auth_code is stored through the encrypted PII vault
//! when TSP_PII_KEY is configured.

use xingtu_tsp::{db::Db, models::CreateVehicleRequest};

const KEY: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";

#[tokio::test]
async fn terminal_auth_code_is_vaulted_and_verifiable() {
    std::env::set_var("TSP_PII_KEY", KEY);
    std::env::remove_var("TSP_PII_PREVIOUS_KEYS");

    let db = Db::in_memory().await.expect("in-memory db");
    let terminal = db
        .register_jt_terminal("13800138000", "test-model", "secret-auth")
        .await
        .expect("register");
    assert_eq!(terminal.auth_code.as_deref(), Some("secret-auth"));

    let ciphertext = db
        .get_pii("terminal", "13800138000", "auth_code")
        .await
        .expect("vault read")
        .expect("ciphertext");
    assert_ne!(ciphertext, "secret-auth");
    assert!(!ciphertext.contains("secret-auth"));

    assert!(db
        .verify_terminal_auth("13800138000", "secret-auth")
        .await
        .expect("verify"));
    assert!(!db
        .verify_terminal_auth("13800138000", "wrong")
        .await
        .expect("verify wrong"));

    let vehicle = db
        .create_vehicle(CreateVehicleRequest {
            vin: "LSVAA2A41NX777777".to_string(),
            plate_no: "沪A77777".to_string(),
            model: "VaultTest".to_string(),
            color: "white".to_string(),
            energy_type: "EV".to_string(),
            driver_name: "Driver".to_string(),
            driver_phone: "13900139000".to_string(),
            org_name: "VaultOrg".to_string(),
        })
        .await
        .expect("create vehicle");
    assert_eq!(vehicle.vehicle.driver_phone, "13900139000");

    let mut stored = db
        .get_vehicle(&vehicle.vehicle.id)
        .await
        .expect("get vehicle")
        .expect("vehicle");
    assert_eq!(stored.vehicle.driver_phone, "__pii_vault__");
    db.hydrate_vehicle_pii(&mut stored.vehicle)
        .await
        .expect("hydrate");
    assert_eq!(stored.vehicle.driver_phone, "13900139000");
    assert_eq!(stored.vehicle.driver_name, "Driver");
    let ciphertext = db
        .get_pii("vehicle", &vehicle.vehicle.id, "driver_phone")
        .await
        .expect("vault read")
        .expect("ciphertext");
    assert!(!ciphertext.contains("13900139000"));

    let second = db
        .create_vehicle(CreateVehicleRequest {
            vin: "LSVAA2A41NX888888".to_string(),
            plate_no: "娌狝88888".to_string(),
            model: "VaultTest2".to_string(),
            color: "white".to_string(),
            energy_type: "EV".to_string(),
            driver_name: "Driver2".to_string(),
            driver_phone: "13700137000".to_string(),
            org_name: "VaultOrg".to_string(),
        })
        .await
        .expect("create second vehicle");

    let before = db.count_key_events("decrypt").await.expect("decrypt audit");
    let mut page = db
        .list_vehicles(None, None, Some(1), Some(10), "VaultOrg")
        .await
        .expect("list vehicles");
    let mut vehicles: Vec<&mut xingtu_tsp::models::Vehicle> = page
        .items
        .iter_mut()
        .map(|item| &mut item.vehicle)
        .collect();
    db.hydrate_vehicles_pii(&mut vehicles)
        .await
        .expect("batch hydrate");
    let phones: Vec<&str> = page
        .items
        .iter()
        .map(|item| item.vehicle.driver_phone.as_str())
        .collect();
    assert!(phones.contains(&"13900139000"));
    assert!(phones.contains(&"13700137000"));
    assert!(db.count_key_events("decrypt").await.expect("decrypt audit") > before);

    assert!(db.count_key_events("encrypt").await.expect("encrypt audit") >= 3);
    assert!(db.count_key_events("decrypt").await.expect("decrypt audit") >= 1);
    assert_eq!(second.vehicle.driver_phone, "13700137000");

    std::env::remove_var("TSP_PII_KEY");
}

#[tokio::test]
async fn vehicle_pii_erase_removes_vault_and_columns() {
    std::env::set_var("TSP_PII_KEY", KEY);
    let db = Db::in_memory().await.expect("db");
    let vehicle = db
        .create_vehicle(CreateVehicleRequest {
            vin: "LSVAA2A41NX999991".to_string(),
            plate_no: "娌狝99991".to_string(),
            model: "EraseTest".to_string(),
            color: "white".to_string(),
            energy_type: "EV".to_string(),
            driver_name: "Erase Driver".to_string(),
            driver_phone: "13600136000".to_string(),
            org_name: "EraseOrg".to_string(),
        })
        .await
        .expect("vehicle");
    let deleted = db
        .erase_vehicle_pii(&vehicle.vehicle.id)
        .await
        .expect("erase");
    assert_eq!(deleted, 2);
    assert!(db
        .get_pii("vehicle", &vehicle.vehicle.id, "driver_phone")
        .await
        .expect("read")
        .is_none());
    let cleared = db
        .get_vehicle(&vehicle.vehicle.id)
        .await
        .expect("get")
        .expect("vehicle");
    assert_eq!(cleared.vehicle.driver_phone, "");
    assert_eq!(cleared.vehicle.driver_name, "");
}
