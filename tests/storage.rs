mod common;

use xingtu_tsp::{db::Db, models::LocationSample, seed};

#[tokio::test]
async fn sqlite_file_database_seed_and_command_status_work() {
    // Cargo provides a writable scratch directory for integration tests; the
    // process temp directory can be sandboxed in some environments.
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("db-file-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).expect("create test dir");
    let path = dir.join("nested").join("tsp.db");
    let url = path.to_string_lossy().into_owned();

    let db = Db::open(url).await.expect("open file db");
    let seeded = seed::seed_if_empty(&db).await.expect("seed");
    assert!(seeded);
    assert!(db
        .find_user_by_username("admin")
        .await
        .expect("find user")
        .is_some());

    let page = db
        .list_vehicles(None, None, Some(1), Some(50), "*")
        .await
        .expect("list vehicles");
    assert!(page.total >= 10, "seed should create the demo fleet");
    let overview = db.dashboard_overview("*").await.expect("dashboard");
    assert!(overview.fleet.total >= 10);
    assert!(!seed::seed_if_empty(&db).await.expect("reseed"));

    let target = page
        .items
        .iter()
        .find(|detail| !matches!(detail.state.status.as_str(), "offline" | "maintenance"))
        .expect("an online demo vehicle");
    let command = db
        .create_command(&target.vehicle.id, "lock", None, "admin")
        .await
        .expect("create command");
    let sent = db
        .set_command_status(&command.id, "sent", None)
        .await
        .expect("sent")
        .expect("command");
    assert_eq!(sent.status, "sent");
    assert!(sent.sent_at.is_some());
    let done = db
        .set_command_status(&command.id, "success", Some("ok".to_string()))
        .await
        .expect("done")
        .expect("command");
    assert_eq!(done.status, "success");
    assert!(done.completed_at.is_some());

    drop(db);
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn batch_location_ingest_uses_one_result_set() {
    let db = Db::in_memory().await.expect("in-memory db");
    seed::seed_if_empty(&db).await.expect("seed");
    let page = db
        .list_vehicles(None, None, Some(1), Some(10), "*")
        .await
        .expect("vehicles");
    let vehicle_id = page.items[0].vehicle.id.clone();
    let now = chrono::Utc::now();
    let samples: Vec<LocationSample> = [2usize, 0, 1]
        .into_iter()
        .map(|index| LocationSample {
            vehicle_id: vehicle_id.clone(),
            ts: (now + chrono::Duration::seconds(index as i64)).to_rfc3339(),
            lat: 31.23 + index as f64 * 0.0001,
            lon: 121.47,
            speed: 30.0,
            heading: 90.0,
            ignition: true,
            soc: Some(60.0),
            odometer_km: None,
            source: "batch-test".to_string(),
            extra_json: None,
        })
        .collect();
    let timestamps: Vec<String> = samples.iter().map(|sample| sample.ts.clone()).collect();
    let duplicate_batch = samples.clone();
    let outcomes = db.ingest_locations(samples).await.expect("batch ingest");
    assert_eq!(outcomes.len(), 3);
    assert!(outcomes.iter().all(Option::is_some));
    let returned: Vec<String> = outcomes
        .iter()
        .flatten()
        .map(|outcome| outcome.ts.clone())
        .collect();
    let mut sorted = returned.clone();
    sorted.sort();
    assert_eq!(returned, sorted);
    let existing = db
        .existing_telemetry_timestamps(&vehicle_id, &timestamps)
        .await
        .expect("batch existence query");
    assert_eq!(existing.len(), 3);
    let duplicates = db
        .ingest_locations(duplicate_batch)
        .await
        .expect("duplicate batch");
    assert!(duplicates.iter().all(Option::is_none));
}

#[tokio::test]
async fn encrypted_pii_vault_stores_only_ciphertext() {
    let db = Db::in_memory().await.expect("in-memory db");
    db.put_pii("vehicle", "veh-1", "driver_phone", "ciphertext-v1")
        .await
        .expect("put");
    assert_eq!(
        db.get_pii("vehicle", "veh-1", "driver_phone")
            .await
            .expect("get"),
        Some("ciphertext-v1".to_string())
    );
    db.put_pii("vehicle", "veh-1", "driver_phone", "ciphertext-v2")
        .await
        .expect("update");
    assert_eq!(
        db.get_pii("vehicle", "veh-1", "driver_phone")
            .await
            .expect("get"),
        Some("ciphertext-v2".to_string())
    );
    assert_eq!(
        db.delete_pii("vehicle", "veh-1", "driver_phone")
            .await
            .expect("delete"),
        1
    );
    assert_eq!(
        db.get_pii("vehicle", "veh-1", "driver_phone")
            .await
            .expect("get"),
        None
    );
}

#[tokio::test]
async fn pii_and_key_audit_retention_purges_old_rows() {
    let db = Db::in_memory().await.expect("in-memory db");
    db.put_pii("vehicle", "v1", "driver_phone", "cipher")
        .await
        .expect("put");
    db.record_key_event("local-keyring", "encrypt", true)
        .await
        .expect("key event");
    let future = "9999-01-01T00:00:00Z";
    assert_eq!(db.purge_pii_before(future).await.expect("purge pii"), 1);
    assert_eq!(
        db.purge_key_audit_before(future)
            .await
            .expect("purge audit"),
        1
    );
    assert_eq!(
        db.get_pii("vehicle", "v1", "driver_phone")
            .await
            .expect("get"),
        None
    );
}

#[tokio::test]
async fn backfill_claims_works_on_sqlite() {
    let db = Db::in_memory().await.expect("db");
    let count = db.backfill_telemetry_claims().await.expect("backfill");
    assert_eq!(count, 0);
}
