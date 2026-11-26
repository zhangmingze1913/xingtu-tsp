use crate::{
    config::{Config, TimeSeriesBackend},
    db::Db,
    error::{AppError, AppResult},
};

pub(super) fn timeseries(config: &Config) -> AppResult<()> {
    if !config.timeseries_backend_raw.trim().is_empty()
        && TimeSeriesBackend::parse(&config.timeseries_backend_raw).is_none()
    {
        return Err(AppError::internal(
            "TSP_TIMESERIES_BACKEND 只能是 postgres / tdengine / dual",
        ));
    }
    if config.timeseries_backend.uses_tdengine() && config.tdengine_url.is_none() {
        return Err(AppError::internal(
            "TSP_TIMESERIES_BACKEND=tdengine/dual 需要 TSP_TDENGINE_URL",
        ));
    }
    if config.timeseries_backend == TimeSeriesBackend::Postgres && config.tdengine_url.is_some() {
        tracing::info!("TSP_TDENGINE_URL is set but backend is postgres; TDengine is disabled");
    }
    Ok(())
}

pub(super) fn security(config: &Config) -> AppResult<()> {
    if !matches!(
        config.command_mode.as_str(),
        "simulated" | "jt808" | "disabled"
    ) {
        return Err(AppError::internal(
            "TSP_COMMAND_MODE 仅支持 simulated / jt808 / disabled",
        ));
    }
    if !config.production {
        return Ok(());
    }
    if config.demo_mode {
        return Err(AppError::internal("生产环境禁止 TSP_DEMO_MODE=true"));
    }
    if config.jwt_private_key.is_none() {
        return Err(AppError::internal(
            "production requires TSP_JWT_PRIVATE_KEY_FILE (RS256 private key)",
        ));
    }
    if let Some(redis_url) = config.redis_url.as_deref() {
        if redis_url.starts_with("redis://") && !config.redis_allow_plaintext {
            return Err(AppError::internal(
                "production cluster requires rediss:// or TSP_REDIS_ALLOW_PLAINTEXT=true",
            ));
        }
    }
    match (&config.jt808_tls_cert, &config.jt808_tls_key) {
        (Some(_), None) => {
            return Err(AppError::internal(
                "TSP_JT808_TLS_CERT requires TSP_JT808_TLS_KEY",
            ))
        }
        (None, _) if !config.jt808_allow_plaintext => {
            return Err(AppError::internal(
                "production JT808 requires TLS cert/key or TSP_JT808_ALLOW_PLAINTEXT=true",
            ))
        }
        (None, _) => tracing::warn!("production JT808 is running without TLS"),
        _ => {}
    }
    if config.command_mode == "simulated" {
        return Err(AppError::internal(
            "生产环境禁止 TSP_COMMAND_MODE=simulated；请使用 jt808 或 disabled",
        ));
    }
    if config.admin_password.as_deref() == Some("admin123") {
        return Err(AppError::internal(
            "production must not use admin123 as the administrator password",
        ));
    }
    let has_pii_key = std::env::var("TSP_PII_KEY")
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false)
        || std::env::var("TSP_PII_KEY_FILE")
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false);
    let has_kms = std::env::var("TSP_KMS_ENDPOINT")
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false);
    let export_required = std::env::var("TSP_EXPORT_REQUIRE_ENCRYPTION")
        .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes"))
        .unwrap_or(false);
    let has_export_key = [
        "TSP_EXPORT_ENCRYPTION_KEY",
        "TSP_EXPORT_ENCRYPTION_KEYS",
        "TSP_EXPORT_ENCRYPTION_KEY_FILE",
        "TSP_EXPORT_ENCRYPTION_KEYS_FILE",
    ]
    .iter()
    .any(|name| {
        std::env::var(name)
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false)
    });
    validate_production_secrets(has_pii_key, has_kms, export_required, has_export_key)?;
    Ok(())
}

/// Pure production-secret policy, split from env reads so it is testable
/// without mutating process-global environment variables.
fn validate_production_secrets(
    has_pii_key: bool,
    has_kms: bool,
    export_required: bool,
    has_export_key: bool,
) -> AppResult<()> {
    if !has_pii_key && !has_kms {
        return Err(AppError::internal(
            "production requires TSP_PII_KEY/TSP_PII_KEY_FILE or TSP_KMS_ENDPOINT",
        ));
    }
    if !export_required {
        return Err(AppError::internal(
            "production requires TSP_EXPORT_REQUIRE_ENCRYPTION=true",
        ));
    }
    if !has_export_key {
        return Err(AppError::internal(
            "production requires an export encryption key (TSP_EXPORT_ENCRYPTION_KEY/KEYS or file)",
        ));
    }
    Ok(())
}

pub(super) async fn production_database(config: &Config, db: &Db) -> AppResult<()> {
    if config.production && db.has_default_admin_password().await? {
        return Err(AppError::internal(
            "生产数据库仍存在 admin/admin123 默认账号，请先修改管理员密码",
        ));
    }
    Ok(())
}

pub(super) fn cluster(config: &Config) -> AppResult<()> {
    if !config.cluster_mode {
        return Ok(());
    }
    if !config.is_postgres() && !config.is_mysql() {
        return Err(AppError::internal(
            "集群模式必须使用 PostgreSQL 或 MySQL；请将 TSP_DATABASE 设置为 postgres:// 或 mysql:// URL",
        ));
    }
    if config.redis_url.is_none() {
        return Err(AppError::internal(
            "集群模式必须配置 TSP_REDIS_URL（Redis 事件总线/会话/选主）",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::validate_production_secrets;

    #[test]
    fn production_secret_policy_rejects_plaintext_paths() {
        assert!(validate_production_secrets(false, false, true, true).is_err());
        assert!(validate_production_secrets(true, false, false, true).is_err());
        assert!(validate_production_secrets(true, false, true, false).is_err());
        assert!(validate_production_secrets(true, false, true, true).is_ok());
        assert!(validate_production_secrets(false, true, true, true).is_ok());
    }
}
