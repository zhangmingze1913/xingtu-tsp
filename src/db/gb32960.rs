//! GB/T 32960 device registry.

use sqlx::Row;

use crate::{error::AppResult, util::now_iso};

use super::Db;

impl Db {
    pub async fn set_gb32960_device(&self, vin: &str, enabled: bool, note: &str) -> AppResult<()> {
        let vin = vin.trim().to_ascii_uppercase();
        let mut tx = self.pool().begin().await?;
        self.query("DELETE FROM gb32960_devices WHERE vin = ?1")
            .bind(&vin)
            .execute(&mut tx)
            .await?;
        self.query(
            "INSERT INTO gb32960_devices (vin, enabled, note, created_at)
             VALUES (?1, ?2, ?3, ?4)",
        )
        .bind(&vin)
        .bind(if enabled { 1i64 } else { 0i64 })
        .bind(note)
        .bind(now_iso())
        .execute(&mut tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn gb32960_device_enabled(&self, vin: &str) -> AppResult<Option<bool>> {
        let vin = vin.trim().to_ascii_uppercase();
        let row = self
            .query("SELECT enabled FROM gb32960_devices WHERE vin = ?1")
            .bind(vin)
            .fetch_optional(self.pool())
            .await?;
        Ok(row.map(|row| row.get::<i64, _>(0) != 0))
    }

    pub async fn list_gb32960_devices(&self) -> AppResult<Vec<(String, bool, String)>> {
        let rows = self
            .query("SELECT vin, enabled, note FROM gb32960_devices ORDER BY vin")
            .fetch_all(self.pool())
            .await?;
        Ok(rows
            .iter()
            .map(|row| {
                (
                    row.get::<String, _>(0),
                    row.get::<i64, _>(1) != 0,
                    row.get::<String, _>(2),
                )
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn device_registry_toggles_access() {
        let db = Db::in_memory().await.expect("db");
        assert_eq!(
            db.gb32960_device_enabled("LSVAA329600000009")
                .await
                .unwrap(),
            None
        );
        db.set_gb32960_device("lsvaa329600000009", true, "fleet")
            .await
            .unwrap();
        assert_eq!(
            db.gb32960_device_enabled("LSVAA329600000009")
                .await
                .unwrap(),
            Some(true)
        );
        db.set_gb32960_device("LSVAA329600000009", false, "revoked")
            .await
            .unwrap();
        assert_eq!(
            db.gb32960_device_enabled("LSVAA329600000009")
                .await
                .unwrap(),
            Some(false)
        );
        assert_eq!(db.list_gb32960_devices().await.unwrap().len(), 1);
    }
}
