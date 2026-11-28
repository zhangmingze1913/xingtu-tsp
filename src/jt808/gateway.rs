//! JT/T 808 TCP gateway runtime: accept loop, per-connection framing and
//! message dispatch. Wire codec lives in `codec`; platform downlink helpers
//! live in the parent module.

use std::{
    fs::File,
    io::BufReader,
    net::SocketAddr,
    str::FromStr,
    sync::Arc,
    time::{Duration, Instant},
};

use ipnet::IpNet;
use memchr::memchr;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::{sync::OwnedSemaphorePermit, sync::Semaphore};
use tokio_rustls::TlsAcceptor;

use super::*;

mod dispatch;
mod downlink;
mod location;
mod registration;
mod terminal_profile;
use dispatch::process_frame;
use location::{ingest_batch, ingest_location};

pub async fn serve(state: AppState) -> crate::error::AppResult<()> {
    let listener = TcpListener::bind(&state.config.jt808_addr)
        .await
        .map_err(|error| {
            tracing::error!(%error, addr = %state.config.jt808_addr, "JT808 bind failed");
            crate::error::AppError::internal("JT/T 808 网关端口监听失败")
        })?;
    let tls = load_tls(&state.config)?;
    let allowed = load_allowlist(&state.config)?;
    if tls.is_none() && !state.config.jt808_allow_plaintext {
        return Err(crate::error::AppError::internal(
            "JT808 TLS is not configured and plaintext is disabled",
        ));
    }
    tracing::info!(
        addr = %state.config.jt808_addr,
        tls = tls.is_some(),
        plaintext = tls.is_none(),
        "JT/T 808 gateway listening"
    );
    let serial = Arc::new(AtomicU16::new(1));
    let connection_limit = Arc::new(Semaphore::new(state.config.jt808_max_connections.max(1)));

    loop {
        let (stream, peer) = match listener.accept().await {
            Ok(value) => value,
            Err(error) => {
                tracing::warn!(%error, "JT808 accept failed");
                continue;
            }
        };
        if !peer_allowed(peer, &allowed) {
            tracing::warn!(%peer, "JT808 peer rejected by CIDR allowlist");
            continue;
        }
        let permit = match connection_limit.clone().try_acquire_owned() {
            Ok(permit) => permit,
            Err(_) => {
                state
                    .metrics
                    .jt808_rejected_connections
                    .fetch_add(1, Ordering::Relaxed);
                tracing::warn!(%peer, limit = state.config.jt808_max_connections, "JT808 connection limit reached");
                drop(stream);
                continue;
            }
        };
        let state = state.clone();
        let serial = serial.clone();
        let peer = peer.to_string();
        let tls = tls.clone();
        tokio::spawn(async move {
            let _permit: OwnedSemaphorePermit = permit;
            tracing::debug!(%peer, "JT808 terminal connected");
            state
                .metrics
                .jt808_connections
                .fetch_add(1, Ordering::Relaxed);
            let result = match tls {
                Some(acceptor) => match acceptor.accept(stream).await {
                    Ok(stream) => {
                        handle_connection(stream, state.clone(), serial, peer.clone()).await
                    }
                    Err(error) => {
                        tracing::warn!(%peer, %error, "JT808 TLS handshake failed");
                        Ok(())
                    }
                },
                None => handle_connection(stream, state.clone(), serial, peer.clone()).await,
            };
            if let Err(error) = result {
                tracing::warn!(%peer, %error, "JT808 connection ended with error");
            }
            state
                .metrics
                .jt808_connections
                .fetch_sub(1, Ordering::Relaxed);
        });
    }
}

fn load_tls(config: &crate::config::Config) -> crate::error::AppResult<Option<TlsAcceptor>> {
    let Some(cert_path) = config.jt808_tls_cert.as_deref() else {
        return Ok(None);
    };
    let key_path = config.jt808_tls_key.as_deref().ok_or_else(|| {
        crate::error::AppError::internal("TSP_JT808_TLS_CERT requires TSP_JT808_TLS_KEY")
    })?;
    let _ = tokio_rustls::rustls::crypto::ring::default_provider().install_default();
    let mut cert_reader = BufReader::new(File::open(cert_path).map_err(|error| {
        tracing::error!(%error, path = %cert_path.display(), "open JT808 TLS cert failed");
        crate::error::AppError::internal("JT808 TLS 证书读取失败")
    })?);
    let certs = rustls_pemfile::certs(&mut cert_reader)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| crate::error::AppError::internal("JT808 TLS 证书格式错误"))?;
    let mut key_reader = BufReader::new(File::open(key_path).map_err(|error| {
        tracing::error!(%error, path = %key_path.display(), "open JT808 TLS key failed");
        crate::error::AppError::internal("JT808 TLS 私钥读取失败")
    })?);
    let key = rustls_pemfile::private_key(&mut key_reader)
        .map_err(|_| crate::error::AppError::internal("JT808 TLS 私钥格式错误"))?
        .ok_or_else(|| crate::error::AppError::internal("JT808 TLS 私钥为空"))?;
    let server = if let Some(ca_path) = config.jt808_client_ca.as_deref() {
        let mut ca_reader = BufReader::new(File::open(ca_path).map_err(|error| {
            tracing::error!(%error, path = %ca_path.display(), "open JT808 client CA failed");
            crate::error::AppError::internal("JT808 客户端 CA 读取失败")
        })?);
        let mut roots = tokio_rustls::rustls::RootCertStore::empty();
        for cert in rustls_pemfile::certs(&mut ca_reader)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| crate::error::AppError::internal("JT808 客户端 CA 格式错误"))?
        {
            roots
                .add(cert)
                .map_err(|_| crate::error::AppError::internal("JT808 客户端 CA 无效"))?;
        }
        let verifier = tokio_rustls::rustls::server::WebPkiClientVerifier::builder(Arc::new(roots))
            .build()
            .map_err(|_| crate::error::AppError::internal("JT808 mTLS 校验器初始化失败"))?;
        tokio_rustls::rustls::ServerConfig::builder()
            .with_client_cert_verifier(verifier)
            .with_single_cert(certs, key)
            .map_err(|_| crate::error::AppError::internal("JT808 mTLS 配置失败"))?
    } else {
        tokio_rustls::rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(certs, key)
            .map_err(|_| crate::error::AppError::internal("JT808 TLS 配置失败"))?
    };
    tracing::info!(
        cert = %cert_path.display(),
        mtls = config.jt808_client_ca.is_some(),
        "JT808 TLS enabled"
    );
    Ok(Some(TlsAcceptor::from(Arc::new(server))))
}

fn load_allowlist(config: &crate::config::Config) -> crate::error::AppResult<Vec<IpNet>> {
    config
        .jt808_allowed_cidrs
        .iter()
        .map(|value| {
            IpNet::from_str(value).map_err(|_| {
                crate::error::AppError::internal(format!("invalid JT808 CIDR: {value}"))
            })
        })
        .collect()
}

fn peer_allowed(peer: SocketAddr, allowed: &[IpNet]) -> bool {
    allowed.is_empty() || allowed.iter().any(|network| network.contains(&peer.ip()))
}

const MAX_JT808_BUFFER_BYTES: usize = 64 * 1024;

async fn handle_connection<S>(
    stream: S,
    state: AppState,
    serial: Arc<AtomicU16>,
    peer: String,
) -> Result<(), ProtocolError>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (mut reader, mut writer) = tokio::io::split(stream);
    let cancel = Arc::new(AtomicBool::new(false));
    let (out_tx, mut out_rx) = mpsc::channel::<Vec<u8>>(SESSION_OUTBOX_CAPACITY);
    let writer_task = tokio::spawn(async move {
        while let Some(frame) = out_rx.recv().await {
            if writer.write_all(&frame).await.is_err() {
                break;
            }
        }
    });
    let mut buffer: Vec<u8> = Vec::with_capacity(1024);
    let mut fragments: HashMap<(u16, u16), Vec<Option<Vec<u8>>>> = HashMap::new();
    let mut fragment_started: HashMap<(u16, u16), Instant> = HashMap::new();
    let max_fragmented_messages = 16usize;
    let max_fragment_bytes = 256 * 1024usize;
    let fragment_ttl = Duration::from_secs(30);
    let mut authenticated = false;
    let mut sessions: HashSet<String> = HashSet::new();

    loop {
        if cancel.load(Ordering::Relaxed) {
            tracing::info!(%peer, "JT808 session superseded; closing old connection");
            break;
        }
        let mut chunk = [0u8; 2048];
        let read =
            match tokio::time::timeout(Duration::from_secs(180), reader.read(&mut chunk)).await {
                Ok(Ok(read)) => read,
                Ok(Err(_)) => break,
                Err(_) => break,
            };
        if read == 0 {
            break;
        }
        if cancel.load(Ordering::Relaxed) {
            tracing::info!(%peer, "JT808 session superseded; closing old connection");
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
        // An unterminated frame must not grow the per-connection buffer without
        // bound. A valid 808 frame is ~2 KiB after escaping, so 64 KiB is a
        // generous guard against a slow/malicious sender.
        if buffer.len() > MAX_JT808_BUFFER_BYTES {
            tracing::warn!(%peer, bytes = buffer.len(), "JT808 frame buffer limit exceeded");
            break;
        }

        // Walk the frames inside the read buffer with a cursor and compact the
        // buffer once per read. Draining after every frame moved the whole tail
        // each time, which is O(n^2) when one read carries many small messages.
        let mut cursor = 0usize;
        loop {
            let Some(rel_start) = memchr(0x7e, &buffer[cursor..]) else {
                cursor = buffer.len();
                break;
            };
            let start = cursor + rel_start;
            let Some(rel_end) = memchr(0x7e, &buffer[start + 1..]).map(|offset| offset + 1) else {
                cursor = start;
                break;
            };
            let end = start + rel_end;
            if end <= start + 1 {
                // Empty frame (`7e 7e`): drop it without allocating.
                cursor = end + 1;
                continue;
            }

            // Parse straight out of the read buffer; `parse_frame` returns an
            // owned frame, so no full-frame copy is needed per message.
            let parsed = parse_frame(&buffer[start..=end]);
            cursor = end + 1;
            let frame = match parsed {
                Ok(frame) => frame,
                Err(error) => {
                    state
                        .metrics
                        .jt808_frame_errors
                        .fetch_add(1, Ordering::Relaxed);
                    tracing::debug!(%error, "JT808 frame parse failed");
                    continue;
                }
            };
            state.metrics.jt808_frames.fetch_add(1, Ordering::Relaxed);
            let frame = if let Some((total, index)) = frame.subpackage {
                if total > 64 || index > total {
                    tracing::debug!(total, index, "JT808 invalid sub-package metadata");
                    continue;
                }
                let key = (frame.msg_id, frame.serial);
                let now = Instant::now();
                let stale: Vec<_> = fragment_started
                    .iter()
                    .filter(|(_, started)| now.duration_since(**started) > fragment_ttl)
                    .map(|(key, _)| *key)
                    .collect();
                for key in stale {
                    fragments.remove(&key);
                    fragment_started.remove(&key);
                }
                if !fragments.contains_key(&key) && fragments.len() >= max_fragmented_messages {
                    tracing::debug!(?key, "too many concurrent JT808 fragmented messages");
                    continue;
                }
                let pending_bytes: usize = fragments
                    .values()
                    .flat_map(|parts| parts.iter().flatten())
                    .map(Vec::len)
                    .sum();
                if pending_bytes + frame.body.len() > max_fragment_bytes {
                    tracing::debug!("JT808 fragment memory budget exceeded");
                    fragments.remove(&key);
                    fragment_started.remove(&key);
                    continue;
                }
                fragment_started.entry(key).or_insert(now);
                let slot = fragments
                    .entry(key)
                    .or_insert_with(|| vec![None; total as usize]);
                if slot.len() != total as usize {
                    fragments.remove(&key);
                    fragment_started.remove(&key);
                    continue;
                }
                slot[(index - 1) as usize] = Some(frame.body);
                if slot.iter().all(Option::is_some) {
                    let body = slot
                        .iter_mut()
                        .filter_map(Option::take)
                        .flatten()
                        .collect::<Vec<u8>>();
                    fragments.remove(&key);
                    fragment_started.remove(&key);
                    Frame {
                        body,
                        subpackage: None,
                        ..frame
                    }
                } else {
                    continue;
                }
            } else {
                frame
            };
            if let Some(response) = process_frame(
                &state,
                &frame,
                &serial,
                &mut authenticated,
                &peer,
                &mut sessions,
                &out_tx,
                &cancel,
            )
            .await
            {
                if out_tx.try_send(response).is_err() {
                    break;
                }
            }
        }
        if cursor > 0 {
            buffer.drain(..cursor);
        }
    }

    for phone in sessions {
        state.cluster.unregister_session(&phone, &peer).await;
        unregister_terminal_sender(&phone, &cancel);
    }
    drop(out_tx);
    let _ = writer_task.await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_roundtrip() {
        let body = vec![0x01, 0x02, 0x7e, 0x7d, 0x03];
        let raw = build_frame(0x0200, "013800000001", 42, &body).unwrap();
        assert_eq!(raw.first(), Some(&0x7e));
        assert_eq!(raw.last(), Some(&0x7e));
        let parsed = parse_frame(&raw).unwrap();
        assert_eq!(parsed.msg_id, 0x0200);
        assert_eq!(parsed.terminal_id, "013800000001");
        assert_eq!(parsed.serial, 42);
        assert_eq!(parsed.body, body);
    }

    #[test]
    fn rejects_bad_checksum() {
        let mut raw = build_frame(0x0002, "013800000001", 1, &[]).unwrap();
        let middle = raw.len() / 2;
        raw[middle] ^= 0xff;
        assert!(parse_frame(&raw).is_err());
    }

    #[test]
    fn parses_location_body() {
        let mut body = Vec::new();
        body.extend_from_slice(&0u32.to_be_bytes());
        body.extend_from_slice(&1u32.to_be_bytes());
        body.extend_from_slice(&(31_230_400u32).to_be_bytes());
        body.extend_from_slice(&(121_473_700u32).to_be_bytes());
        body.extend_from_slice(&12u16.to_be_bytes());
        body.extend_from_slice(&(563u16).to_be_bytes());
        body.extend_from_slice(&88u16.to_be_bytes());
        body.extend_from_slice(&[0x26, 0x11, 0x15, 0x20, 0x10, 0x11]);
        body.extend_from_slice(&[0x01, 0x04, 0x00, 0x00, 0x30, 0x39]);
        body.extend_from_slice(&[0x04, 0x04, 0x00, 0x00, 0x00, 0x09]);
        body.extend_from_slice(&[0x25, 0x04, 0x00, 0x00, 0x00, 0x03]);
        body.extend_from_slice(&[0x11, 0x05, 0x01, 0x00, 0x00, 0x00, 0x07]);
        body.extend_from_slice(&[0x12, 0x06, 0x02, 0x00, 0x00, 0x00, 0x08, 0x01]);
        body.extend_from_slice(&[0x13, 0x08, 0x04, 0x00, 0x00, 0x00, 0x09, 0x00, 0x3c, 0x01]);
        let location = parse_location(&body).unwrap();
        assert!((location.lat - 31.2304).abs() < 1e-9);
        assert!((location.lon - 121.4737).abs() < 1e-9);
        assert!((location.speed - 56.3).abs() < 1e-9);
        assert_eq!(location.heading, 88.0);
        assert!(location.ignition);
        assert_eq!(location.mileage_km, Some(1234.5));
        assert!(location
            .extra_json
            .as_deref()
            .is_some_and(|value| value.contains("01")));
        assert_eq!(location.ts, "2026-11-15T12:10:11Z");
    }

    #[test]
    fn response_body_contains_request_serial() {
        let frame = Frame {
            msg_id: MSG_HEARTBEAT,
            terminal_id: "013800000001".into(),
            serial: 77,
            body: vec![],
            subpackage: None,
        };
        let raw = general_response(&frame, 0, 9).unwrap();
        let parsed = parse_frame(&raw).unwrap();
        assert_eq!(parsed.msg_id, MSG_GENERAL_RESPONSE);
        assert_eq!(parsed.body, vec![0x00, 0x4d, 0x00]);
    }

    fn location_body() -> Vec<u8> {
        let mut body = Vec::new();
        body.extend_from_slice(&0u32.to_be_bytes());
        body.extend_from_slice(&1u32.to_be_bytes());
        body.extend_from_slice(&(31_230_400u32).to_be_bytes());
        body.extend_from_slice(&(121_473_700u32).to_be_bytes());
        body.extend_from_slice(&12u16.to_be_bytes());
        body.extend_from_slice(&(563u16).to_be_bytes());
        body.extend_from_slice(&88u16.to_be_bytes());
        body.extend_from_slice(&[0x26, 0x11, 0x15, 0x20, 0x10, 0x11]);
        body
    }

    #[test]
    fn registration_requires_full_2019_body() {
        assert!(parse_registration(&[0u8; 29]).is_err());
        let mut body = Vec::new();
        body.extend_from_slice(&[0x00, 0x1f, 0x00, 0x01]);
        body.extend_from_slice(b"TESTM");
        body.extend_from_slice(&[b'M'; 20]);
        body.extend_from_slice(b"1234567");
        body.push(1);
        let parsed = parse_registration(&body).unwrap();
        assert_eq!(parsed.manufacturer, "TESTM");
        assert_eq!(parsed.province, 31);
        assert_eq!(parsed.city, 1);
        assert_eq!(parsed.terminal_id, "31323334353637");
        assert_eq!(parsed.plate_color, 1);
    }

    #[test]
    fn parses_batch_location() {
        let one = location_body();
        let mut body = Vec::new();
        body.extend_from_slice(&2u16.to_be_bytes());
        body.push(0);
        for _ in 0..2 {
            body.extend_from_slice(&(one.len() as u16).to_be_bytes());
            body.extend_from_slice(&one);
        }
        let points = parse_location_batch(&body).unwrap();
        assert_eq!(points.len(), 2);
        assert!((points[0].lat - 31.2304).abs() < 1e-9);
    }

    #[test]
    fn terminal_parameter_roundtrip() {
        let params = vec![
            TerminalParameter {
                id: 0x0000_0001,
                value: vec![30],
            },
            TerminalParameter {
                id: 0x0000_0018,
                value: b"127.0.0.1:8080".to_vec(),
            },
        ];
        let body = encode_terminal_parameters(&params).unwrap();
        assert_eq!(decode_terminal_parameters(&body).unwrap(), params);
        assert_eq!(encode_terminal_control(1, &[7]), vec![1, 7]);
        assert_eq!(encode_temp_tracking(10, 60), vec![0, 10, 0, 0, 0, 60]);
        assert_eq!(
            encode_specific_parameter_query(&[1, 2]).unwrap(),
            vec![2, 0, 0, 0, 1, 0, 0, 0, 2]
        );
    }

    #[test]
    fn vehicle_control_action_profile() {
        assert_eq!(vehicle_control_action("lock"), Some((0x01, vec![0x01])));
        assert_eq!(vehicle_control_action("unlock"), Some((0x01, vec![0x00])));
        assert_eq!(vehicle_control_action("start"), Some((0x02, vec![0x01])));
        assert_eq!(vehicle_control_action("unknown"), None);
    }

    #[test]
    fn downlink_tracking_marks_success() {
        let phone = "013800000099";
        track_downlink(phone, MSG_SET_PARAMETERS, 4242);
        assert_eq!(
            downlink_status(phone, 4242).map(|status| status.status),
            Some("pending".to_string())
        );
        let done = complete_downlink(phone, 4242, 0).expect("tracked downlink");
        assert_eq!(done.status, "success");
        assert_eq!(done.result, Some(0));
    }

    #[test]
    fn location_query_response_is_parseable() {
        let body =
            encode_location_response(31.2304, 121.4737, 42.5, 88.0, true, "2026-11-16T10:00:00Z");
        let parsed = parse_location(&body).unwrap();
        assert!((parsed.lat - 31.2304).abs() < 1e-6);
        assert!((parsed.lon - 121.4737).abs() < 1e-6);
        assert!((parsed.speed - 42.5).abs() < 1e-6);
        assert!(parsed.ignition);
    }

    #[test]
    fn cidr_allowlist_accepts_and_rejects_peers() {
        use std::net::IpAddr;
        let allowed: Vec<ipnet::IpNet> = vec!["10.0.0.0/8".parse().unwrap()];
        let inside = std::net::SocketAddr::new(IpAddr::from([10, 1, 2, 3]), 18080);
        let outside = std::net::SocketAddr::new(IpAddr::from([192, 168, 1, 1]), 18080);
        assert!(peer_allowed(inside, &allowed));
        assert!(!peer_allowed(outside, &allowed));
        assert!(peer_allowed(outside, &[]));
    }

    #[test]
    fn rejects_out_of_range_location() {
        let mut body = location_body();
        body[8..12].copy_from_slice(&(91_000_000u32).to_be_bytes());
        assert!(parse_location(&body).is_err());
    }

    #[test]
    fn rejects_truncated_location_extra() {
        let mut body = location_body();
        body.extend_from_slice(&[0x01, 0x04, 0x00]);
        assert!(parse_location(&body).is_err());
    }

    #[test]
    fn rejects_oversized_batch_body() {
        let body = vec![0u8; crate::jt808::MAX_BATCH_BODY_BYTES + 1];
        assert!(parse_location_batch(&body).is_err());
    }

    #[test]
    fn rejects_oversized_location_batch() {
        let mut body = Vec::new();
        body.extend_from_slice(&((crate::jt808::MAX_BATCH_LOCATIONS + 1) as u16).to_be_bytes());
        body.push(0);
        assert!(parse_location_batch(&body).is_err());
    }

    #[test]
    fn parses_2019_candidate_registration_and_attributes() {
        let mut body = Vec::new();
        body.extend_from_slice(&31u16.to_be_bytes());
        body.extend_from_slice(&1u16.to_be_bytes());
        body.extend_from_slice(b"MANUFACTUR1");
        body.extend_from_slice(&[b'M'; 30]);
        body.extend_from_slice(&[b'T'; 30]);
        body.push(1);
        let (plate, _, _) = encoding_rs::GBK.encode("沪A12345");
        body.extend_from_slice(&plate);
        assert_eq!(body.len(), 76 + plate.len());
        let registration = parse_registration(&body).unwrap();
        assert_eq!(registration.layout, "2019-table8");
        assert_eq!(registration.manufacturer, "MANUFACTUR1");
        assert_eq!(registration.plate_color, 1);
        assert_eq!(registration.plate_no, "沪A12345");

        let mut attributes = Vec::new();
        attributes.extend_from_slice(&1u16.to_be_bytes());
        attributes.extend_from_slice(b"MANUF");
        attributes.extend_from_slice(&[b'M'; 30]);
        attributes.extend_from_slice(&[b'T'; 30]);
        attributes.extend_from_slice(&[0x89, 0x01, 0x23, 0x45, 0x67, 0x89, 0x01, 0x23, 0x45, 0x67]);
        attributes.push(3);
        attributes.extend_from_slice(b"HW1");
        attributes.push(3);
        attributes.extend_from_slice(b"FW1");
        attributes.push(0x03);
        attributes.push(0x01);
        let parsed = parse_terminal_attributes(&attributes).unwrap();
        assert_eq!(parsed.layout, "2019-table20");
        assert_eq!(parsed.manufacturer, "MANUF");
        assert_eq!(parsed.iccid, "89012345678901234567");
        assert_eq!(parsed.hardware_version, "HW1");
        assert_eq!(parsed.firmware_version, "FW1");
        assert_eq!(parsed.gnss, Some(3));
    }

    fn fixture_hex(value: &str) -> Vec<u8> {
        let value: String = value.chars().filter(|ch| !ch.is_whitespace()).collect();
        (0..value.len())
            .step_by(2)
            .map(|index| u8::from_str_radix(&value[index..index + 2], 16).expect("hex"))
            .collect()
    }

    #[test]
    fn synthetic_0100_0107_0200_8203_fixtures() {
        let registration = parse_registration(&fixture_hex(include_str!(
            "../../tests/fixtures/standards/jt808_0100_2019.hex"
        )))
        .expect("0100 fixture");
        assert_eq!(registration.layout, "2019-table8");
        assert_eq!(registration.plate_no, "沪A12345");

        let attributes = parse_terminal_attributes(&fixture_hex(include_str!(
            "../../tests/fixtures/standards/jt808_0107_2019.hex"
        )))
        .expect("0107 fixture");
        assert_eq!(attributes.iccid, "89012345678901234567");
        assert_eq!(attributes.hardware_version, "HW1");

        let location = parse_location(&fixture_hex(include_str!(
            "../../tests/fixtures/standards/jt808_0200_mandatory.hex"
        )))
        .expect("0200 fixture");
        let extras = location.extra_json.expect("extras");
        for key in [
            "mileage_km",
            "fuel_l",
            "driving_record_km",
            "alarm_id",
            "overspeed_area_id",
            "area_direction",
            "route_time_seconds",
            "extended_signal",
            "signal",
            "satellites",
        ] {
            assert!(extras.contains(key), "missing {key}");
        }

        let alarm = fixture_hex(include_str!(
            "../../tests/fixtures/standards/jt808_8203.hex"
        ));
        assert_eq!(alarm, encode_alarm_ack(7, 9));
    }
}
