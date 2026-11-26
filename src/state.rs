use std::{
    collections::HashMap,
    sync::{Arc, Mutex, RwLock},
    time::{Duration, Instant},
};

use tokio::sync::Mutex as AsyncMutex;

use crate::{
    auth_keys::JwtKeys, cluster::Cluster, config::Config, db::Db, error::AppResult,
    iggy_bus::IggyBus, metrics::Metrics, realtime::EventBus, tdengine::TdengineStore,
    timeseries::TsinkStore, walrus_bus::WalrusBus,
};

/// Application dependency container.
///
/// Fields are crate-visible: handlers use them, but the public library surface
/// is the constructor/builder rather than the transport/store types.
#[derive(Clone)]
pub struct AppState {
    pub db: Db,
    pub(crate) config: Arc<Config>,
    pub(crate) bus: EventBus,
    pub(crate) cluster: Cluster,
    pub(crate) metrics: Arc<Metrics>,
    pub(crate) tsink: Option<TsinkStore>,
    pub(crate) tdengine: Option<TdengineStore>,
    pub(crate) dashboard_cache: Arc<DashboardCache>,
    pub(crate) jwt: Arc<JwtKeys>,
    revoked_tokens: Arc<TokenRevocations>,
}

struct TokenRevocations {
    revoked: RwLock<HashMap<String, Instant>>,
    valid: RwLock<HashMap<String, Instant>>,
}

impl TokenRevocations {
    fn new() -> Self {
        Self {
            revoked: RwLock::new(HashMap::new()),
            valid: RwLock::new(HashMap::new()),
        }
    }

    fn revoke(&self, jti: &str, ttl: Duration) {
        if let Ok(mut entries) = self.revoked.write() {
            if entries.len() > 8192 {
                entries.clear();
            }
            entries.insert(jti.to_string(), Instant::now() + ttl);
        }
        if let Ok(mut valid) = self.valid.write() {
            valid.remove(jti);
        }
    }

    fn mark_valid(&self, jti: &str, ttl: Duration) {
        if let Ok(mut valid) = self.valid.write() {
            if valid.len() > 8192 {
                valid.clear();
            }
            valid.insert(jti.to_string(), Instant::now() + ttl);
        }
    }

    fn status(&self, jti: &str) -> Option<bool> {
        let now = Instant::now();
        if let Ok(mut entries) = self.revoked.write() {
            entries.retain(|_, expires| *expires > now);
            if entries.contains_key(jti) {
                return Some(true);
            }
        }
        if let Ok(mut valid) = self.valid.write() {
            valid.retain(|_, expires| *expires > now);
            if valid.contains_key(jti) {
                return Some(false);
            }
        }
        None
    }
}

pub(crate) struct DashboardCache {
    ttl: Duration,
    entries: RwLock<HashMap<String, (Instant, String)>>,
    inflight: Mutex<HashMap<String, Arc<AsyncMutex<()>>>>,
}

impl DashboardCache {
    fn new(ttl: Duration) -> Self {
        Self {
            ttl,
            entries: RwLock::new(HashMap::new()),
            inflight: Mutex::new(HashMap::new()),
        }
    }

    /// Per-organization single-flight guard used to coalesce cold cache
    /// refreshes without serializing unrelated organizations.
    pub(crate) fn lock_for(&self, org: &str) -> Arc<AsyncMutex<()>> {
        let mut inflight = match self.inflight.lock() {
            Ok(inflight) => inflight,
            Err(poisoned) => poisoned.into_inner(),
        };
        if inflight.len() > 64 {
            inflight.clear();
        }
        inflight
            .entry(org.to_string())
            .or_insert_with(|| Arc::new(AsyncMutex::new(())))
            .clone()
    }

    pub(crate) fn get(&self, org: &str) -> Option<String> {
        let entries = self.entries.read().ok()?;
        let (at, body) = entries.get(org)?;
        if at.elapsed() <= self.ttl {
            Some(body.clone())
        } else {
            None
        }
    }

    pub(crate) fn put(&self, org: &str, body: String) {
        if let Ok(mut entries) = self.entries.write() {
            if entries.len() > 64 {
                entries.clear();
            }
            entries.insert(org.to_string(), (Instant::now(), body));
        }
    }

    pub(crate) fn clear(&self) {
        if let Ok(mut entries) = self.entries.write() {
            entries.clear();
        }
    }
}

/// Builder for [`AppState`]. Keeps transport/store wiring in one place and
/// avoids three near-identical constructors.
pub(crate) struct AppStateBuilder {
    db: Db,
    config: Config,
    cluster: Cluster,
    bus: EventBus,
    tsink: Option<TsinkStore>,
    tdengine: Option<TdengineStore>,
}

impl AppStateBuilder {
    pub(crate) fn new(config: Config, db: Db) -> Self {
        let cluster = Cluster::disabled(&config.node_id);
        let bus = EventBus::new(512);
        Self {
            db,
            config,
            cluster,
            bus,
            tsink: None,
            tdengine: None,
        }
    }

    pub(crate) fn cluster(mut self, cluster: Cluster) -> Self {
        self.bus = EventBus::with_cluster(512, &cluster);
        self.cluster = cluster;
        self
    }

    pub(crate) fn cluster_and_iggy(
        mut self,
        cluster: Cluster,
        iggy: IggyBus,
        incoming: tokio::sync::mpsc::UnboundedReceiver<String>,
    ) -> Self {
        self.bus = EventBus::with_cluster_and_iggy(512, &cluster, iggy, incoming);
        self.cluster = cluster;
        self
    }

    pub(crate) fn tsink(mut self, tsink: Option<TsinkStore>) -> Self {
        self.tsink = tsink;
        self
    }

    pub(crate) fn tdengine(mut self, tdengine: Option<TdengineStore>) -> Self {
        self.tdengine = tdengine;
        self
    }

    pub(crate) fn walrus(mut self, walrus: WalrusBus) -> Self {
        self.bus.attach_walrus(walrus);
        self
    }

    pub(crate) fn build(self) -> AppState {
        self.try_build().expect("invalid application state")
    }

    pub(crate) fn try_build(self) -> AppResult<AppState> {
        let jwt = JwtKeys::from_config(&self.config)?;
        Ok(AppState {
            db: self.db,
            config: Arc::new(self.config),
            bus: self.bus,
            cluster: self.cluster,
            metrics: Arc::new(Metrics::default()),
            tsink: self.tsink,
            tdengine: self.tdengine,
            dashboard_cache: Arc::new(DashboardCache::new(Duration::from_secs(2))),
            jwt,
            revoked_tokens: Arc::new(TokenRevocations::new()),
        })
    }
}

impl AppState {
    /// Local/test profile: no Redis coordination, singleton scheduler.
    pub fn new(config: Config, db: Db) -> Self {
        Self::builder(config, db).build()
    }

    pub(crate) async fn is_access_revoked(&self, jti: &str) -> AppResult<bool> {
        if let Some(status) = self.revoked_tokens.status(jti) {
            return Ok(status);
        }
        if !self.cluster.enabled() {
            return Ok(false);
        }
        let revoked = self.cluster.is_token_revoked(jti).await?;
        if revoked {
            self.revoked_tokens.revoke(jti, Duration::from_secs(900));
        } else {
            // Short positive cache: cuts Redis EXISTS on hot token paths while
            // bounding cross-node logout propagation to this window.
            self.revoked_tokens.mark_valid(jti, Duration::from_secs(5));
        }
        Ok(revoked)
    }

    pub(crate) async fn revoke_access(&self, jti: &str, ttl: Duration) -> AppResult<()> {
        self.revoked_tokens.revoke(jti, ttl);
        if self.cluster.enabled() {
            self.cluster.revoke_token(jti, ttl.as_secs().max(1)).await?;
        }
        Ok(())
    }

    pub(crate) fn builder(config: Config, db: Db) -> AppStateBuilder {
        AppStateBuilder::new(config, db)
    }
}
