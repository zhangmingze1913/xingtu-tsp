//! Vehicle queries and PII hydration.
//!
//! List/detail reads return a vault marker in the denormalized columns. PII is
//! hydrated in one batch query and one audit batch, not per row, so a 50-row
//! page performs one vault read rather than 50. Keep that shape when adding
//! fields.

use sqlx::Row;

use crate::{
    error::{AppError, AppResult},
    models::{
        CreateVehicleRequest, Page, UpdateVehicleRequest, Vehicle, VehicleDetail, VehicleState,
    },
    security::crypto::provider_from_env,
    util::{now_iso, page_bounds},
};

use super::{
    get_i64, map_state, map_terminal_at, map_vehicle, Db, STATE_COLS, TERMINAL_COLS, VEHICLE_COLS,
};

const DRIVER_PHONE_VAULT_MARKER: &str = "__pii_vault__";

fn vehicle_keyring() -> Option<std::sync::Arc<dyn crate::security::crypto::KeyProvider>> {
    use std::sync::{Arc, Mutex, OnceLock};
    type CachedKeyring = Option<(String, Arc<dyn crate::security::crypto::KeyProvider>)>;
    static CACHE: OnceLock<Mutex<CachedKeyring>> = OnceLock::new();
    let marker = format!(
        "{}|{}",
        std::env::var("TSP_PII_KEY").unwrap_or_default(),
        std::env::var("TSP_PII_KEY_FILE").unwrap_or_default()
    );
    let cache = CACHE.get_or_init(|| Mutex::new(None));
    let mut guard = cache.lock().ok()?;
    if let Some((cached, provider)) = guard.as_ref() {
        if *cached == marker {
            return Some(provider.clone());
        }
    }
    let provider = provider_from_env()?;
    *guard = Some((marker, provider.clone()));
    Some(provider)
}

impl Db {
    pub async fn list_vehicles(
        &self,
        search: Option<String>,
        status: Option<String>,
        page: Option<i64>,
        page_size: Option<i64>,
        org: &str,
    ) -> AppResult<Page<VehicleDetail>> {
        let (page, page_size, offset) = page_bounds(page, page_size);
        let search = search.unwrap_or_default();
        let status = status.unwrap_or_default();

        // MySQL's `||` is logical OR unless PIPES_AS_CONCAT is enabled, so the
        // canonical SQLite/PostgreSQL concatenation becomes CONCAT there.
        let search_match = if self.is_mysql() {
            "v.plate_no LIKE CONCAT('%', ?1, '%') OR v.vin LIKE CONCAT('%', ?1, '%')
             OR v.driver_name LIKE CONCAT('%', ?1, '%')"
        } else {
            "v.plate_no LIKE '%' || ?1 || '%' OR v.vin LIKE '%' || ?1 || '%'
             OR v.driver_name LIKE '%' || ?1 || '%'"
        };

        let count_query = format!(
            "SELECT COUNT(*) FROM vehicles v
             LEFT JOIN vehicle_state s ON s.vehicle_id = v.id
             WHERE (?1 = '' OR {search_match})
               AND (?2 = '' OR s.status = ?2)
               AND (?3 = '*' OR v.org_name = ?3)"
        );
        let count_sql = self.sql(&count_query);
        let count_row = self
            .query(&count_sql)
            .bind(search.as_str())
            .bind(status.as_str())
            .bind(org)
            .fetch_one(self.pool())
            .await?;
        let total = get_i64(&count_row, 0)?;

        let query = format!(
            "SELECT {VEHICLE_COLS}, {STATE_COLS}, {TERMINAL_COLS}
             FROM vehicles v
             LEFT JOIN vehicle_state s ON s.vehicle_id = v.id
             LEFT JOIN terminals t ON t.vehicle_id = v.id
             WHERE (?1 = '' OR {search_match})
               AND (?2 = '' OR s.status = ?2)
               AND (?3 = '*' OR v.org_name = ?3)
             ORDER BY v.plate_no
             LIMIT ?4 OFFSET ?5"
        );
        let sql = self.sql(&query);
        let rows = self
            .query(&sql)
            .bind(search.as_str())
            .bind(status.as_str())
            .bind(org)
            .bind(page_size)
            .bind(offset)
            .fetch_all(self.pool())
            .await?;
        let mut items = Vec::with_capacity(rows.len());
        for row in &rows {
            let vehicle = map_vehicle(row)?;
            let state = map_state(row, 10)?;
            let has_terminal: Option<String> = row.try_get(20)?;
            let terminal = if has_terminal.is_some() {
                Some(map_terminal_at(row, 20)?)
            } else {
                None
            };
            items.push(VehicleDetail {
                vehicle,
                state,
                terminal,
            });
        }
        Ok(Page {
            items,
            total,
            page,
            page_size,
        })
    }

    pub async fn get_vehicle(&self, id: &str) -> AppResult<Option<VehicleDetail>> {
        let query = format!(
            "SELECT {VEHICLE_COLS}, {STATE_COLS}, {TERMINAL_COLS}
             FROM vehicles v
             LEFT JOIN vehicle_state s ON s.vehicle_id = v.id
             LEFT JOIN terminals t ON t.vehicle_id = v.id
             WHERE v.id = ?1"
        );
        let sql = self.sql(&query);
        let row = self
            .query(&sql)
            .bind(id)
            .fetch_optional(self.pool())
            .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let vehicle = map_vehicle(&row)?;
        let state = map_state(&row, 10)?;
        let has_terminal: Option<String> = row.try_get(20)?;
        let terminal = if has_terminal.is_some() {
            Some(map_terminal_at(&row, 20)?)
        } else {
            None
        };
        Ok(Some(VehicleDetail {
            vehicle,
            state,
            terminal,
        }))
    }

    pub async fn create_vehicle(&self, req: CreateVehicleRequest) -> AppResult<VehicleDetail> {
        let id = uuid::Uuid::new_v4().to_string();
        let ts = now_iso();
        let mut vehicle = Vehicle {
            id: id.clone(),
            vin: req.vin.trim().to_uppercase(),
            plate_no: req.plate_no.trim().to_string(),
            model: req.model.trim().to_string(),
            color: req.color.trim().to_string(),
            energy_type: req.energy_type.trim().to_uppercase(),
            driver_name: req.driver_name.trim().to_string(),
            driver_phone: req.driver_phone.trim().to_string(),
            org_name: req.org_name.trim().to_string(),
            created_at: ts.clone(),
        };
        if vehicle.vin.chars().count() != 17 {
            return Err(AppError::bad_request("VIN 必须为 17 位"));
        }
        if vehicle.plate_no.is_empty() || vehicle.model.is_empty() {
            return Err(AppError::bad_request("车牌号与车型不能为空"));
        }
        if !matches!(vehicle.energy_type.as_str(), "EV" | "HEV" | "PHEV" | "ICE") {
            return Err(AppError::bad_request(
                "能源类型仅支持 EV / HEV / PHEV / ICE",
            ));
        }

        let duplicate_sql =
            self.sql("SELECT COUNT(*) FROM vehicles WHERE vin = ?1 OR plate_no = ?2");
        let duplicate_row = self
            .query(&duplicate_sql)
            .bind(vehicle.vin.as_str())
            .bind(vehicle.plate_no.as_str())
            .fetch_one(self.pool())
            .await?;
        if get_i64(&duplicate_row, 0)? > 0 {
            return Err(AppError::conflict("VIN 或车牌号已存在"));
        }

        let original_phone = vehicle.driver_phone.clone();
        let original_name = vehicle.driver_name.clone();
        let vaulted = if let Some(ring) = vehicle_keyring() {
            let phone_ciphertext = ring
                .encrypt(&original_phone)
                .map_err(|error| AppError::internal(error.to_string()))?;
            let name_ciphertext = ring
                .encrypt(&original_name)
                .map_err(|error| AppError::internal(error.to_string()))?;
            self.put_pii("vehicle", &vehicle.id, "driver_phone", &phone_ciphertext)
                .await?;
            self.put_pii("vehicle", &vehicle.id, "driver_name", &name_ciphertext)
                .await?;
            self.record_key_event(ring.provider_name(), "encrypt", true)
                .await?;
            vehicle.driver_phone = DRIVER_PHONE_VAULT_MARKER.to_string();
            vehicle.driver_name = DRIVER_PHONE_VAULT_MARKER.to_string();
            true
        } else {
            false
        };
        let mut tx = self.pool().begin().await?;
        let insert_sql = self.sql(
            "INSERT INTO vehicles
             (id, vin, plate_no, model, color, energy_type, driver_name, driver_phone, org_name, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        );
        self.query(&insert_sql)
            .bind(&vehicle.id)
            .bind(&vehicle.vin)
            .bind(&vehicle.plate_no)
            .bind(&vehicle.model)
            .bind(&vehicle.color)
            .bind(&vehicle.energy_type)
            .bind(&vehicle.driver_name)
            .bind(&vehicle.driver_phone)
            .bind(&vehicle.org_name)
            .bind(&vehicle.created_at)
            .execute(&mut tx)
            .await?;
        let state_sql = self.sql(
            "INSERT INTO vehicle_state
             (vehicle_id, lat, lon, speed, heading, ignition, soc, odometer_km, status, updated_at)
             VALUES (?1, NULL, NULL, 0, 0, 0, 0, 0, 'offline', ?2)",
        );
        self.query(&state_sql)
            .bind(&vehicle.id)
            .bind(&ts)
            .execute(&mut tx)
            .await?;
        tx.commit().await?;
        if vaulted {
            vehicle.driver_phone = original_phone;
            vehicle.driver_name = original_name;
        }

        Ok(VehicleDetail {
            vehicle,
            state: VehicleState {
                vehicle_id: id,
                lat: None,
                lon: None,
                speed: 0.0,
                heading: 0.0,
                ignition: false,
                soc: 0.0,
                odometer_km: 0.0,
                status: "offline".to_string(),
                updated_at: ts,
            },
            terminal: None,
        })
    }

    pub async fn update_vehicle_and_get(
        &self,
        id: &str,
        req: UpdateVehicleRequest,
    ) -> AppResult<Option<VehicleDetail>> {
        let mut req = req;
        if let Some(ring) = vehicle_keyring() {
            if let Some(phone) = req.driver_phone.take() {
                let ciphertext = ring
                    .encrypt(&phone)
                    .map_err(|error| AppError::internal(error.to_string()))?;
                self.put_pii("vehicle", id, "driver_phone", &ciphertext)
                    .await?;
                req.driver_phone = Some(DRIVER_PHONE_VAULT_MARKER.to_string());
            }
            if let Some(name) = req.driver_name.take() {
                let ciphertext = ring
                    .encrypt(&name)
                    .map_err(|error| AppError::internal(error.to_string()))?;
                self.put_pii("vehicle", id, "driver_name", &ciphertext)
                    .await?;
                self.record_key_event(ring.provider_name(), "encrypt", true)
                    .await?;
                req.driver_name = Some(DRIVER_PHONE_VAULT_MARKER.to_string());
            }
        }
        let sql = self.sql(
            "UPDATE vehicles SET
                plate_no = COALESCE(?2, plate_no),
                model = COALESCE(?3, model),
                color = COALESCE(?4, color),
                energy_type = COALESCE(?5, energy_type),
                driver_name = COALESCE(?6, driver_name),
                driver_phone = COALESCE(?7, driver_phone),
                org_name = COALESCE(?8, org_name)
             WHERE id = ?1",
        );
        let result = self
            .query(&sql)
            .bind(id)
            .bind(req.plate_no)
            .bind(req.model)
            .bind(req.color)
            .bind(req.energy_type)
            .bind(req.driver_name)
            .bind(req.driver_phone)
            .bind(req.org_name)
            .execute(self.pool())
            .await?;
        if result.rows_affected() == 0 {
            return Ok(None);
        }
        self.get_vehicle(id).await
    }

    pub async fn hydrate_vehicle_pii(&self, vehicle: &mut crate::models::Vehicle) -> AppResult<()> {
        let mut one = [vehicle];
        self.hydrate_vehicles_pii(&mut one).await
    }

    /// Batch PII hydration: one vault query for all vehicles, one audit
    /// transaction for all decryptions.
    pub async fn hydrate_vehicles_pii(
        &self,
        vehicles: &mut [&mut crate::models::Vehicle],
    ) -> AppResult<()> {
        let Some(ring) = vehicle_keyring() else {
            return Ok(());
        };
        let ids: Vec<String> = vehicles
            .iter()
            .filter(|v| {
                v.driver_phone == DRIVER_PHONE_VAULT_MARKER
                    || v.driver_name == DRIVER_PHONE_VAULT_MARKER
            })
            .map(|v| v.id.clone())
            .collect();
        if ids.is_empty() {
            return Ok(());
        }
        let values = self
            .get_pii_batch("vehicle", &ids, &["driver_phone", "driver_name"])
            .await?;
        let mut events = Vec::new();
        for vehicle in vehicles {
            if vehicle.driver_phone == DRIVER_PHONE_VAULT_MARKER {
                if let Some(ciphertext) =
                    values.get(&(vehicle.id.clone(), "driver_phone".to_string()))
                {
                    let decrypted = ring.decrypt(ciphertext);
                    events.push(decrypted.is_ok());
                    if let Ok(phone) = decrypted {
                        vehicle.driver_phone = phone;
                    }
                }
            }
            if vehicle.driver_name == DRIVER_PHONE_VAULT_MARKER {
                if let Some(ciphertext) =
                    values.get(&(vehicle.id.clone(), "driver_name".to_string()))
                {
                    let decrypted = ring.decrypt(ciphertext);
                    events.push(decrypted.is_ok());
                    if let Ok(name) = decrypted {
                        vehicle.driver_name = name;
                    }
                }
            }
        }
        self.record_key_event_outcome(ring.provider_name(), "decrypt", &events)
            .await?;
        Ok(())
    }

    pub async fn delete_vehicle(&self, id: &str) -> AppResult<bool> {
        let sql = self.sql("DELETE FROM vehicles WHERE id = ?1");
        let result = self.query(&sql).bind(id).execute(self.pool()).await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn find_vehicle_id_by_vin(&self, vin: &str) -> AppResult<Option<String>> {
        let row = self
            .query("SELECT id FROM vehicles WHERE vin = ?1")
            .bind(vin)
            .fetch_optional(self.pool())
            .await?;
        Ok(row.map(|row| row.get::<String, _>(0)))
    }

    pub async fn get_vehicle_plate(&self, id: &str) -> AppResult<Option<String>> {
        let sql = self.sql("SELECT plate_no FROM vehicles WHERE id = ?1");
        let row = self
            .query(&sql)
            .bind(id)
            .fetch_optional(self.pool())
            .await?;
        match row {
            Some(row) => Ok(Some(row.try_get(0)?)),
            None => Ok(None),
        }
    }

    pub async fn all_vehicle_ids(&self) -> AppResult<Vec<(String, String)>> {
        let rows = sqlx::query("SELECT id, plate_no FROM vehicles ORDER BY plate_no")
            .fetch_all(self.pool())
            .await?;
        let mut items = Vec::with_capacity(rows.len());
        for row in &rows {
            items.push((row.try_get(0)?, row.try_get(1)?));
        }
        Ok(items)
    }

    pub async fn seed_vehicle(&self, vehicle: &Vehicle, state: &VehicleState) -> AppResult<()> {
        let mut tx = self.pool().begin().await?;
        let insert_sql = self.sql(
            "INSERT INTO vehicles
             (id, vin, plate_no, model, color, energy_type, driver_name, driver_phone, org_name, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        );
        self.query(&insert_sql)
            .bind(&vehicle.id)
            .bind(&vehicle.vin)
            .bind(&vehicle.plate_no)
            .bind(&vehicle.model)
            .bind(&vehicle.color)
            .bind(&vehicle.energy_type)
            .bind(&vehicle.driver_name)
            .bind(&vehicle.driver_phone)
            .bind(&vehicle.org_name)
            .bind(&vehicle.created_at)
            .execute(&mut tx)
            .await?;
        let state_sql = self.sql(
            "INSERT INTO vehicle_state
             (vehicle_id, lat, lon, speed, heading, ignition, soc, odometer_km, status, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        );
        self.query(&state_sql)
            .bind(&state.vehicle_id)
            .bind(state.lat)
            .bind(state.lon)
            .bind(state.speed)
            .bind(state.heading)
            .bind(state.ignition as i64)
            .bind(state.soc)
            .bind(state.odometer_km)
            .bind(&state.status)
            .bind(&state.updated_at)
            .execute(&mut tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }
}
