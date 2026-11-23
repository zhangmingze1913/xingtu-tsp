use crate::{
    config::Config,
    db::Db,
    error::{AppError, AppResult},
};

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
            "生产环境禁止使用 admin123 作为管理员密码",
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
