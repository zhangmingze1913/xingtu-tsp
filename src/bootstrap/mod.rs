//! Startup composition root.
//!
//! Keeps process-level wiring (database, seed, cluster, event bus, optional
//! stores) out of the HTTP/worker section in `lib.rs`.

use crate::{
    config::Config,
    db::Db,
    error::{AppError, AppResult},
    seed,
    state::AppState,
};

mod cluster;
mod validate;

pub(crate) async fn bootstrap(config: &Config) -> AppResult<AppState> {
    validate::cluster(config)?;
    validate::security(config)?;

    let db = Db::open_with_postgis(&config.database_url, config.postgis_enabled).await?;
    let seeded = seed_database(config, &db).await?;
    if seeded {
        if config.demo_mode {
            tracing::info!("demo dataset seeded (admin / admin123)");
        } else {
            tracing::info!("administrator account initialized");
        }
    }
    if let Err(error) = db.backfill_telemetry_claims().await {
        tracing::warn!(%error, "telemetry claim backfill failed");
    }
    if seeded {
        db.backfill_postgis().await?;
    }
    let _ = db.prune_refresh_tokens(&crate::util::now_iso()).await;
    validate::production_database(config, &db).await?;
    prune_on_start(&db).await;

    let cluster = cluster::connect(config).await?;
    crate::spawn_gateway_command_subscriber(&cluster);
    crate::spawn_jt808_kick_subscriber(&cluster);

    let mut builder = AppState::builder(config.clone(), db);
    if let Some(iggy_url) = config.iggy_url.as_deref() {
        if !config.cluster_mode {
            return Err(AppError::internal(
                "TSP_IGGY_URL 仅在 TSP_CLUSTER_MODE=true 时生效",
            ));
        }
        let (iggy, incoming) = crate::iggy_bus::IggyBus::connect(
            iggy_url,
            &config.node_id,
            &config.iggy_stream,
            &config.iggy_topic,
            &config.iggy_username,
            &config.iggy_password,
        )
        .await?;
        tracing::info!(
            node = %config.node_id,
            stream = %config.iggy_stream,
            topic = %config.iggy_topic,
            "durable event bus enabled (Iggy)"
        );
        builder = builder.cluster_and_iggy(cluster, iggy, incoming);
    } else if config.cluster_mode {
        builder = builder.cluster(cluster);
    }

    let tsink = if let Some(url) = config.tsink_url.as_deref() {
        Some(crate::timeseries::TsinkStore::start(
            url.to_string(),
            config.tsink_token.clone(),
        )?)
    } else {
        None
    };
    builder = builder.tsink(tsink);

    let tdengine = if let Some(url) = config.tdengine_url.as_deref() {
        Some(
            crate::tdengine::TdengineStore::start(
                url.to_string(),
                config.tdengine_user.clone(),
                config.tdengine_password.clone(),
                config.tdengine_database.clone(),
            )
            .await?,
        )
    } else {
        None
    };
    builder = builder.tdengine(tdengine);

    if let Some(addr) = config.walrus_addr.as_deref() {
        let walrus = crate::walrus_bus::WalrusBus::connect(addr, &config.walrus_topic).await?;
        tracing::info!(%addr, topic = %config.walrus_topic, "Walrus event archive enabled");
        builder = builder.walrus(walrus);
    }

    builder.try_build()
}

async fn seed_database(config: &Config, db: &Db) -> AppResult<bool> {
    if config.demo_mode {
        return match seed::seed_if_empty(db).await {
            Ok(seeded) => Ok(seeded),
            Err(error) if config.cluster_mode => {
                if db.user_count().await.unwrap_or(0) > 0 {
                    tracing::warn!(%error, "seed race lost; another replica initialized the database");
                    Ok(false)
                } else {
                    Err(error)
                }
            }
            Err(error) => Err(error),
        };
    }
    if db.user_count().await? > 0 {
        return Ok(false);
    }
    let Some(password) = config.admin_password.as_deref() else {
        return Err(AppError::internal(
            "生产模式没有用户，请设置 TSP_ADMIN_PASSWORD 初始化管理员",
        ));
    };
    seed::ensure_admin(db, "admin", password).await
}

async fn prune_on_start(db: &Db) {
    let cutoff = (chrono::Utc::now() - chrono::Duration::days(7))
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let _ = db.prune_telemetry_before(&cutoff).await;
    let audit_cutoff = (chrono::Utc::now() - chrono::Duration::days(180))
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let _ = db.prune_audit_before(&audit_cutoff).await;
    let policy = crate::security::retention::RetentionPolicy::default();
    let _ = db.purge_pii_before(&policy.pii_cutoff()).await;
    let _ = db.purge_key_audit_before(&policy.key_audit_cutoff()).await;
}
