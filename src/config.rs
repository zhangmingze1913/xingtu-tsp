use std::{
    env,
    path::{Path, PathBuf},
};

/// Time-series backend selected at deployment time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimeSeriesBackend {
    /// PostgreSQL/PostGIS only (small and medium deployments).
    Postgres,
    /// TDengine is the query store; PostgreSQL remains authoritative.
    Tdengine,
    /// Write PG and TDengine, query TDengine first (migration/large).
    Dual,
}

impl TimeSeriesBackend {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "postgres" | "postgresql" | "pg" => Some(Self::Postgres),
            "tdengine" | "taos" => Some(Self::Tdengine),
            "dual" | "both" => Some(Self::Dual),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Postgres => "postgres",
            Self::Tdengine => "tdengine",
            Self::Dual => "dual",
        }
    }

    pub fn uses_tdengine(self) -> bool {
        !matches!(self, Self::Postgres)
    }
}

#[derive(Clone, Debug)]
pub struct Config {
    pub bind: String,
    /// Raw `TSP_DATABASE` value. Either a SQLite filesystem path (local
    /// development, tests and single-node deployments), a `postgres://` /
    /// `postgresql://` URL (cluster deployment), or a `mysql://` / `mariadb://`
    /// URL (single-node MySQL-compatible deployment).
    pub database_url: String,
    /// SQLite path derived from [`Config::database_url`]. It is empty when the
    /// configured database is PostgreSQL or MySQL, because the file path is
    /// unused then.
    pub database_path: PathBuf,
    pub web_dir: PathBuf,
    /// RS256 private key PEM (inline). Prefer `TSP_JWT_PRIVATE_KEY_FILE`.
    pub jwt_private_key: Option<String>,
    /// RS256 public key PEM. Derived from the private key when omitted.
    pub jwt_public_key: Option<String>,
    pub jwt_key_id: String,
    pub jwt_issuer: String,
    pub jwt_audience: String,
    pub jwt_access_ttl_minutes: i64,
    pub jwt_refresh_ttl_hours: i64,
    /// Previous public keys as `kid=path` entries; kept for JWKS rotation.
    pub jwt_previous_keys: Vec<String>,
    pub jt808_addr: String,
    pub demo_mode: bool,
    /// Enables Redis coordination and enforces a shared PostgreSQL database.
    pub cluster_mode: bool,
    pub redis_url: Option<String>,
    pub node_id: String,
    pub node_role: String,
    /// Optional Apache Iggy connection string. When set in cluster mode,
    /// Iggy carries the durable domain event log instead of Redis Pub/Sub.
    pub iggy_url: Option<String>,
    pub iggy_stream: String,
    pub iggy_topic: String,
    pub iggy_username: String,
    pub iggy_password: String,
    /// Optional tsink HTTP time-series store.
    pub tsink_url: Option<String>,
    pub tsink_token: Option<String>,
    /// Optional TDengine REST SQL endpoint.
    pub tdengine_url: Option<String>,
    pub tdengine_user: String,
    pub tdengine_password: String,
    pub tdengine_database: String,
    /// Explicit time-series backend selection.
    pub timeseries_backend: TimeSeriesBackend,
    /// Raw env value, kept for startup validation.
    pub timeseries_backend_raw: String,
    /// Optional Walrus distributed-log TCP endpoint.
    pub walrus_addr: Option<String>,
    pub walrus_topic: String,
    /// OpenRaft control-plane options.
    pub raft_enabled: bool,
    pub raft_node_id: u64,
    pub raft_addr: String,
    pub raft_peers: String,
    pub raft_bootstrap: bool,
    /// Production hardening switches.
    pub production: bool,
    pub cors_origins: Vec<String>,
    pub admin_password: Option<String>,
    pub command_mode: String,
    /// Enable PostGIS geometry columns/spatial queries when the PostgreSQL
    /// server has the extension available. Ignored for SQLite/MySQL.
    pub postgis_enabled: bool,
    /// Require `rediss://` in production unless explicitly allowed.
    pub redis_allow_plaintext: bool,
    /// JT808 TLS certificate/key (PEM paths). Without them the gateway uses
    /// plain TCP only when `jt808_allow_plaintext` is true.
    pub jt808_tls_cert: Option<PathBuf>,
    pub jt808_tls_key: Option<PathBuf>,
    pub jt808_allow_plaintext: bool,
    /// PEM CA used to require client certificates (mTLS) on JT808.
    pub jt808_client_ca: Option<PathBuf>,
    /// Optional CIDR allowlist for JT808 peers.
    pub jt808_allowed_cidrs: Vec<String>,
}

impl Config {
    pub fn from_env() -> Self {
        let _ = dotenvy::dotenv();

        let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let default_sqlite = manifest_dir.join("data").join("tsp.db");
        let database_url = env::var("TSP_DATABASE")
            .unwrap_or_else(|_| default_sqlite.to_string_lossy().into_owned());
        let database_path = sqlite_path_from(&database_url);

        let web_dir = env::var("TSP_WEB_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| manifest_dir.join("web"));

        let node_id = env::var("TSP_NODE_ID")
            .or_else(|_| env::var("HOSTNAME"))
            .unwrap_or_else(|_| {
                format!("node-{}", &uuid::Uuid::new_v4().simple().to_string()[..8])
            });
        let production = env::var("TSP_ENV")
            .map(|value| value.eq_ignore_ascii_case("production"))
            .unwrap_or(false)
            || env::var("TSP_PRODUCTION")
                .map(|value| matches!(value.as_str(), "1" | "true" | "on" | "yes"))
                .unwrap_or(false);
        let demo_mode = env::var("TSP_DEMO_MODE")
            .map(|v| !matches!(v.as_str(), "0" | "false" | "off"))
            .unwrap_or(true);
        let command_mode = env::var("TSP_COMMAND_MODE").unwrap_or_else(|_| {
            if demo_mode {
                "simulated".to_string()
            } else {
                "disabled".to_string()
            }
        });
        let raft_node_id = env::var("TSP_RAFT_NODE_ID")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or_else(|| stable_node_id(&node_id));

        let tdengine_url = env::var("TSP_TDENGINE_URL").ok().filter(|v| !v.is_empty());
        let timeseries_backend_raw = env::var("TSP_TIMESERIES_BACKEND").unwrap_or_default();
        let timeseries_backend = if timeseries_backend_raw.trim().is_empty() {
            if tdengine_url.is_some() {
                TimeSeriesBackend::Dual
            } else {
                TimeSeriesBackend::Postgres
            }
        } else {
            TimeSeriesBackend::parse(&timeseries_backend_raw).unwrap_or(TimeSeriesBackend::Postgres)
        };

        Self {
            bind: env::var("TSP_BIND").unwrap_or_else(|_| "127.0.0.1:8090".to_string()),
            database_url,
            database_path,
            web_dir,
            jwt_private_key: env_file("TSP_JWT_PRIVATE_KEY_FILE").or_else(|| {
                env::var("TSP_JWT_PRIVATE_KEY")
                    .ok()
                    .filter(|v| !v.is_empty())
            }),
            jwt_public_key: env_file("TSP_JWT_PUBLIC_KEY_FILE").or_else(|| {
                env::var("TSP_JWT_PUBLIC_KEY")
                    .ok()
                    .filter(|v| !v.is_empty())
            }),
            jwt_key_id: env::var("TSP_JWT_KEY_ID").unwrap_or_else(|_| "tsp-rs256-1".to_string()),
            jwt_issuer: env::var("TSP_JWT_ISSUER").unwrap_or_else(|_| "xingtu-tsp".to_string()),
            jwt_audience: env::var("TSP_JWT_AUDIENCE").unwrap_or_else(|_| "xingtu-api".to_string()),
            jwt_access_ttl_minutes: env::var("TSP_JWT_ACCESS_TTL_MINUTES")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(15),
            jwt_refresh_ttl_hours: env::var("TSP_JWT_REFRESH_TTL_HOURS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(168),
            jwt_previous_keys: env::var("TSP_JWT_PREVIOUS_KEYS")
                .ok()
                .map(|value| {
                    value
                        .split(',')
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(ToString::to_string)
                        .collect()
                })
                .unwrap_or_default(),
            jt808_addr: env::var("TSP_JT808_ADDR")
                .unwrap_or_else(|_| "127.0.0.1:18080".to_string()),
            demo_mode,
            cluster_mode: env::var("TSP_CLUSTER_MODE")
                .map(|v| matches!(v.as_str(), "1" | "true" | "on" | "yes"))
                .unwrap_or(false),
            redis_url: env::var("TSP_REDIS_URL").ok().filter(|v| !v.is_empty()),
            node_id,
            node_role: env::var("TSP_NODE_ROLE").unwrap_or_else(|_| "api".to_string()),
            iggy_url: env::var("TSP_IGGY_URL").ok().filter(|v| !v.is_empty()),
            iggy_stream: env::var("TSP_IGGY_STREAM").unwrap_or_else(|_| "xingtu-tsp".to_string()),
            iggy_topic: env::var("TSP_IGGY_TOPIC").unwrap_or_else(|_| "events".to_string()),
            iggy_username: env::var("TSP_IGGY_USERNAME").unwrap_or_else(|_| "iggy".to_string()),
            iggy_password: env::var("TSP_IGGY_PASSWORD").unwrap_or_else(|_| "iggy".to_string()),
            tsink_url: env::var("TSP_TSINK_URL").ok().filter(|v| !v.is_empty()),
            tsink_token: env::var("TSP_TSINK_TOKEN").ok().filter(|v| !v.is_empty()),
            tdengine_url,
            tdengine_user: env::var("TSP_TDENGINE_USER").unwrap_or_else(|_| "root".to_string()),
            tdengine_password: env::var("TSP_TDENGINE_PASSWORD")
                .unwrap_or_else(|_| "taosdata".to_string()),
            tdengine_database: env::var("TSP_TDENGINE_DATABASE")
                .unwrap_or_else(|_| "tsp".to_string()),
            timeseries_backend,
            timeseries_backend_raw,
            walrus_addr: env::var("TSP_WALRUS_ADDR").ok().filter(|v| !v.is_empty()),
            walrus_topic: env::var("TSP_WALRUS_TOPIC").unwrap_or_else(|_| "tsp-events".to_string()),
            raft_enabled: env::var("TSP_RAFT_ENABLED")
                .map(|v| matches!(v.as_str(), "1" | "true" | "on" | "yes"))
                .unwrap_or(false),
            raft_node_id,
            raft_addr: env::var("TSP_RAFT_ADDR").unwrap_or_else(|_| "127.0.0.1:7001".to_string()),
            raft_peers: env::var("TSP_RAFT_PEERS").unwrap_or_default(),
            raft_bootstrap: env::var("TSP_RAFT_BOOTSTRAP")
                .map(|v| matches!(v.as_str(), "1" | "true" | "on" | "yes"))
                .unwrap_or(true),
            production,
            cors_origins: env::var("TSP_CORS_ORIGINS")
                .unwrap_or_default()
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string)
                .collect(),
            admin_password: env::var("TSP_ADMIN_PASSWORD")
                .ok()
                .filter(|value| !value.is_empty()),
            command_mode,
            postgis_enabled: env::var("TSP_POSTGIS_ENABLED")
                .map(|v| !matches!(v.as_str(), "0" | "false" | "off"))
                .unwrap_or(true),
            redis_allow_plaintext: env::var("TSP_REDIS_ALLOW_PLAINTEXT")
                .map(|v| matches!(v.as_str(), "1" | "true" | "on" | "yes"))
                .unwrap_or(false),
            jt808_tls_cert: env::var("TSP_JT808_TLS_CERT")
                .ok()
                .filter(|v| !v.is_empty())
                .map(PathBuf::from),
            jt808_tls_key: env::var("TSP_JT808_TLS_KEY")
                .ok()
                .filter(|v| !v.is_empty())
                .map(PathBuf::from),
            jt808_allow_plaintext: env::var("TSP_JT808_ALLOW_PLAINTEXT")
                .map(|v| matches!(v.as_str(), "1" | "true" | "on" | "yes"))
                .unwrap_or(false),
            jt808_client_ca: env::var("TSP_JT808_CLIENT_CA")
                .ok()
                .filter(|v| !v.is_empty())
                .map(PathBuf::from),
            jt808_allowed_cidrs: env::var("TSP_JT808_ALLOWED_CIDRS")
                .ok()
                .map(|value| {
                    value
                        .split(',')
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(ToString::to_string)
                        .collect()
                })
                .unwrap_or_default(),
        }
    }

    /// True when [`Config::database_url`] selects PostgreSQL.
    pub fn is_postgres(&self) -> bool {
        is_postgres_url(&self.database_url)
    }

    /// True when [`Config::database_url`] selects MySQL / MariaDB.
    pub fn is_kingbase(&self) -> bool {
        is_kingbase_url(&self.database_url)
    }

    pub fn is_mysql(&self) -> bool {
        is_mysql_url(&self.database_url)
    }
}

/// PostgreSQL URLs are used verbatim; everything else is a SQLite file path
/// unless it is a MySQL/MariaDB or KingbaseES URL.
fn env_file(name: &str) -> Option<String> {
    let path = env::var(name).ok().filter(|value| !value.is_empty())?;
    match std::fs::read_to_string(&path) {
        Ok(value) => Some(value),
        Err(error) => {
            tracing::error!(%error, %path, "failed to read secret file");
            None
        }
    }
}

pub fn is_kingbase_url(database_url: &str) -> bool {
    database_url.starts_with("kingbase://")
        || database_url.starts_with("kingbase8://")
        || database_url.starts_with("kingbasees://")
}

pub fn is_postgres_url(database_url: &str) -> bool {
    database_url.starts_with("postgres://")
        || database_url.starts_with("postgresql://")
        || is_kingbase_url(database_url)
}

/// MySQL-compatible URLs are `mysql://` or `mariadb://`.
pub fn is_mysql_url(database_url: &str) -> bool {
    database_url.starts_with("mysql://") || database_url.starts_with("mariadb://")
}

/// Stable u64 node id for OpenRaft when `TSP_RAFT_NODE_ID` is not supplied.
/// K8s pods with unique `TSP_NODE_ID` therefore do not all become node 1.
pub fn stable_node_id(node_id: &str) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in node_id.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash.max(1)
}

/// Derive the SQLite file path from a `TSP_DATABASE` value.
///
/// Plain paths are returned unchanged. Explicit `sqlite:` URLs are stripped of
/// their scheme and query parameters. PostgreSQL and MySQL URLs yield an empty
/// path.
fn sqlite_path_from(database_url: &str) -> PathBuf {
    if is_postgres_url(database_url) || is_mysql_url(database_url) {
        return PathBuf::new();
    }
    for prefix in ["sqlite://", "sqlite:"] {
        if let Some(rest) = database_url.strip_prefix(prefix) {
            let path = rest.split('?').next().unwrap_or(rest);
            return PathBuf::from(path);
        }
    }
    PathBuf::from(database_url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_database_urls() {
        assert!(is_postgres_url("postgres://user:pw@db:5432/tsp"));
        assert!(is_postgres_url("postgresql://db/tsp"));
        assert!(!is_postgres_url("data/tsp.db"));
        assert!(!is_postgres_url("sqlite://data/tsp.db"));
        assert!(!is_postgres_url("mysql://user:pw@db:3306/tsp"));
        assert!(is_postgres_url("kingbase://user:pw@db:54321/tsp"));
        assert!(is_postgres_url("kingbase8://db:54321/tsp"));
        assert!(is_kingbase_url("kingbasees://db/tsp"));
        assert!(!is_kingbase_url("postgres://db/tsp"));

        assert!(is_mysql_url("mysql://user:pw@db:3306/tsp"));
        assert!(is_mysql_url("mariadb://db/tsp"));
        assert!(!is_mysql_url("postgres://db/tsp"));
        assert!(!is_mysql_url("data/tsp.db"));

        assert_eq!(
            sqlite_path_from("sqlite://data/tsp.db?mode=rwc"),
            PathBuf::from("data/tsp.db")
        );
        assert_eq!(
            sqlite_path_from("data/tsp.db"),
            PathBuf::from("data/tsp.db")
        );
        assert_eq!(sqlite_path_from("postgres://db/tsp"), PathBuf::new());
        assert_eq!(
            sqlite_path_from("mysql://user:pw@db:3306/tsp"),
            PathBuf::new()
        );
        assert_eq!(
            sqlite_path_from("mariadb://user:pw@db:3306/tsp"),
            PathBuf::new()
        );
        assert_eq!(sqlite_path_from("kingbase://db:54321/tsp"), PathBuf::new());
    }

    #[test]
    fn parses_time_series_backends() {
        assert_eq!(
            TimeSeriesBackend::parse("postgres"),
            Some(TimeSeriesBackend::Postgres)
        );
        assert_eq!(
            TimeSeriesBackend::parse("TAOS"),
            Some(TimeSeriesBackend::Tdengine)
        );
        assert_eq!(
            TimeSeriesBackend::parse("both"),
            Some(TimeSeriesBackend::Dual)
        );
        assert_eq!(TimeSeriesBackend::parse("unknown"), None);
        assert!(!TimeSeriesBackend::Postgres.uses_tdengine());
        assert!(TimeSeriesBackend::Tdengine.uses_tdengine());
        assert_eq!(TimeSeriesBackend::Dual.as_str(), "dual");
    }
}
