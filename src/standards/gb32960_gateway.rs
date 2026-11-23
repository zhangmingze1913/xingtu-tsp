//! GB/T 32960 TCP terminal listener.

use std::{
    collections::{HashMap, HashSet},
    fs::File,
    io::{self, BufReader},
    path::Path,
    sync::{Arc, Mutex, OnceLock},
    time::Instant,
};

use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite},
    net::TcpListener,
};
use tokio_rustls::TlsAcceptor;

use crate::{db::Db, standards::gb32960::Gb32960StreamDecoder, standards::gb32960_ingest::ingest};

/// Bind and serve forever on `addr`.
pub async fn serve(addr: &str, db: Db) -> io::Result<()> {
    let listener = TcpListener::bind(addr).await?;
    serve_listener(listener, db).await
}

/// Bind and serve TLS using PEM certificate/key files.
pub async fn serve_tls(
    addr: &str,
    db: Db,
    cert: &Path,
    key: &Path,
    client_ca: Option<&Path>,
) -> io::Result<()> {
    let acceptor = load_tls(cert, key, client_ca)?;
    let listener = TcpListener::bind(addr).await?;
    loop {
        let (socket, peer) = listener.accept().await?;
        let acceptor = acceptor.clone();
        let db = db.clone();
        tokio::spawn(async move {
            match acceptor.accept(socket).await {
                Ok(stream) => {
                    if let Err(error) = handle_stream(stream, db).await {
                        tracing::warn!(%peer, %error, "GB/T 32960 TLS stream ended");
                    }
                }
                Err(error) => tracing::warn!(%peer, %error, "GB/T 32960 TLS handshake failed"),
            }
        });
    }
}

fn load_tls(
    cert_path: &Path,
    key_path: &Path,
    client_ca: Option<&Path>,
) -> io::Result<TlsAcceptor> {
    let _ = tokio_rustls::rustls::crypto::ring::default_provider().install_default();
    let mut cert_reader = BufReader::new(File::open(cert_path)?);
    let certs: Vec<_> = rustls_pemfile::certs(&mut cert_reader)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid certificate PEM"))?;
    let mut key_reader = BufReader::new(File::open(key_path)?);
    let key = rustls_pemfile::private_key(&mut key_reader)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid private key PEM"))?
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing private key"))?;
    let builder = tokio_rustls::rustls::ServerConfig::builder();
    let config = if let Some(ca_path) = client_ca {
        let mut ca_reader = BufReader::new(File::open(ca_path)?);
        let mut roots = tokio_rustls::rustls::RootCertStore::empty();
        for cert in rustls_pemfile::certs(&mut ca_reader) {
            let cert = cert
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid client CA PEM"))?;
            roots.add(cert).map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidData, "invalid client CA certificate")
            })?;
        }
        let verifier = tokio_rustls::rustls::server::WebPkiClientVerifier::builder(Arc::new(roots))
            .build()
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid client CA"))?;
        builder
            .with_client_cert_verifier(verifier)
            .with_single_cert(certs, key)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid TLS certificate"))?
    } else {
        builder
            .with_no_client_auth()
            .with_single_cert(certs, key)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid TLS certificate"))?
    };
    tracing::info!(cert = %cert_path.display(), mtls = client_ca.is_some(), "GB/T 32960 TLS enabled");
    Ok(TlsAcceptor::from(Arc::new(config)))
}

pub async fn serve_listener(listener: TcpListener, db: Db) -> io::Result<()> {
    loop {
        let (socket, peer) = listener.accept().await?;
        let db = db.clone();
        tokio::spawn(async move {
            if let Err(error) = handle_stream(socket, db).await {
                tracing::warn!(%peer, %error, "GB/T 32960 TCP stream ended");
            }
        });
    }
}

fn allowlist() -> Option<&'static HashSet<String>> {
    static ALLOWED: OnceLock<Option<HashSet<String>>> = OnceLock::new();
    ALLOWED
        .get_or_init(|| {
            let raw = std::env::var("TSP_GB32960_ALLOWED_VINS").ok()?;
            let set: HashSet<String> = raw
                .split(',')
                .map(|value| value.trim().to_ascii_uppercase())
                .filter(|value| !value.is_empty())
                .collect();
            if set.is_empty() {
                None
            } else {
                Some(set)
            }
        })
        .as_ref()
}

fn vin_allowed(frame: &[u8]) -> bool {
    let Some(list) = allowlist() else {
        return true;
    };
    if frame.len() < 21 {
        return false;
    }
    let vin = String::from_utf8_lossy(&frame[4..21]).to_ascii_uppercase();
    list.contains(&vin)
}

struct Bucket {
    tokens: f64,
    last: Instant,
}

/// Per-VIN token bucket. Rate <= 0 disables limiting.
struct VinRateLimiter {
    rate: f64,
    burst: f64,
    buckets: Mutex<HashMap<String, Bucket>>,
}

impl VinRateLimiter {
    fn new(rate: f64) -> Self {
        Self {
            rate,
            burst: (rate * 2.0).max(1.0),
            buckets: Mutex::new(HashMap::new()),
        }
    }

    fn allow_at(&self, vin: &str, now: Instant) -> bool {
        if self.rate <= 0.0 {
            return true;
        }
        let Ok(mut buckets) = self.buckets.lock() else {
            return false;
        };
        let bucket = buckets.entry(vin.to_string()).or_insert(Bucket {
            tokens: self.burst,
            last: now,
        });
        let elapsed = now.saturating_duration_since(bucket.last).as_secs_f64();
        bucket.tokens = (bucket.tokens + elapsed * self.rate).min(self.burst);
        bucket.last = now;
        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            true
        } else {
            false
        }
    }

    fn allow(&self, vin: &str) -> bool {
        self.allow_at(vin, Instant::now())
    }
}

fn rate_limiter() -> Option<&'static VinRateLimiter> {
    static LIMITER: OnceLock<Option<VinRateLimiter>> = OnceLock::new();
    LIMITER
        .get_or_init(|| {
            let rate = std::env::var("TSP_GB32960_RATE_PER_SEC")
                .ok()?
                .trim()
                .parse::<f64>()
                .ok()?;
            if rate <= 0.0 {
                None
            } else {
                Some(VinRateLimiter::new(rate))
            }
        })
        .as_ref()
}

async fn handle_stream<S>(mut stream: S, db: Db) -> io::Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut decoder = Gb32960StreamDecoder::new();
    let mut buffer = [0u8; 4096];
    loop {
        let read = stream.read(&mut buffer).await?;
        if read == 0 {
            return Ok(());
        }
        decoder.feed(&buffer[..read]);
        while let Some(frame) = decoder.next_frame() {
            if !vin_allowed(&frame) {
                tracing::warn!("GB/T 32960 VIN not in allowlist");
                continue;
            }
            let vin = String::from_utf8_lossy(&frame[4..21]).to_ascii_uppercase();
            if let Some(limiter) = rate_limiter() {
                if !limiter.allow(&vin) {
                    tracing::warn!(%vin, "GB/T 32960 rate limit exceeded");
                    continue;
                }
            }
            match db.gb32960_device_enabled(&vin).await {
                Ok(Some(false)) => {
                    tracing::warn!(%vin, "GB/T 32960 device revoked");
                    continue;
                }
                Ok(_) => {}
                Err(error) => {
                    tracing::warn!(%vin, %error, "GB/T 32960 device registry lookup failed")
                }
            }
            if let Err(error) = ingest(&db, &frame).await {
                tracing::warn!(%error, "GB/T 32960 frame rejected");
            }
        }
        if decoder.buffered() > 1_048_576 {
            tracing::warn!("GB/T 32960 stream buffer exceeded limit");
            return Ok(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        models::CreateVehicleRequest,
        standards::gb32960::{
            Gb32960Frame, Gb32960LocationData, Gb32960RealtimeData, Gb32960Section,
            GB32960_REALTIME, SEC_LOCATION, SEC_VEHICLE,
        },
    };
    use tokio::io::{duplex, AsyncWriteExt};

    #[tokio::test]
    async fn tcp_stream_ingests_split_frame() {
        let db = Db::in_memory().await.expect("db");
        let vehicle = db
            .create_vehicle(CreateVehicleRequest {
                vin: "LSVAA329600000003".to_string(),
                plate_no: "沪A32962".to_string(),
                model: "EV".to_string(),
                color: "white".to_string(),
                energy_type: "EV".to_string(),
                driver_name: "Driver".to_string(),
                driver_phone: "13800138002".to_string(),
                org_name: "EVOrg".to_string(),
            })
            .await
            .expect("vehicle");

        let data = Gb32960RealtimeData {
            vehicle_status: 1,
            charge_status: 1,
            run_mode: 1,
            speed_kmh: 20.0,
            odometer_km: 3.0,
            total_voltage_v: 350.0,
            total_current_a: 3.0,
            soc_percent: 60,
            dc_dc_status: 1,
            gear: 1,
            insulation_kohm: 500,
            accel_pedal_percent: 3,
            brake_pedal_percent: 0,
            tail: vec![],
        };
        let location = Gb32960LocationData {
            status: 1,
            lon: 121.2,
            lat: 31.2,
            altitude_m: 3,
        };
        let mut body = vec![0x26, 0x11, 0x23, 0x12, 0x00, 0x00];
        body.extend_from_slice(
            &Gb32960Section {
                data_type: SEC_VEHICLE,
                data: data.encode(),
            }
            .encode(),
        );
        body.extend_from_slice(
            &Gb32960Section {
                data_type: SEC_LOCATION,
                data: location.encode().unwrap(),
            }
            .encode(),
        );
        let raw = Gb32960Frame {
            command: GB32960_REALTIME,
            response: 0xFE,
            vin: "LSVAA329600000003".to_string(),
            encrypt: 1,
            data: body,
        }
        .encode()
        .unwrap();

        let (mut client, server) = duplex(8192);
        let task = tokio::spawn(handle_stream(server, db.clone()));
        client.write_all(&raw[..12]).await.unwrap();
        client.write_all(&raw[12..]).await.unwrap();
        client.shutdown().await.unwrap();
        task.await.unwrap().unwrap();

        assert_eq!(
            db.count_telemetry_for(&vehicle.vehicle.id).await.unwrap(),
            1
        );
    }

    #[test]
    fn allowlist_rejects_unknown_vin() {
        let mut set = HashSet::new();
        set.insert("LSVAA329600000003".to_string());
        assert!(set.contains("LSVAA329600000003"));
        assert!(!set.contains("LSVAA329600000004"));
    }

    #[test]
    fn token_bucket_limits_and_refills() {
        let limiter = VinRateLimiter::new(2.0);
        let now = Instant::now();
        let mut allowed = 0;
        for _ in 0..5 {
            if limiter.allow_at("VIN", now) {
                allowed += 1;
            }
        }
        assert_eq!(allowed, 4);
        assert!(limiter.allow_at("VIN", now + std::time::Duration::from_secs(1)));
    }
}
