//! Simulated TLS platform handshake, using test certificates under
//! tests/fixtures/tls. This validates CA/SNI wiring, not regulator certification.

use std::{fs, sync::Arc};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_rustls::{rustls::ServerConfig, TlsAcceptor};

use xingtu_tsp::standards::jt809::{
    self, Jt809Frame, Jt809Login, Jt809SessionState, MSG_LOGIN_RESPONSE,
};

fn fixture(name: &str) -> Vec<u8> {
    fs::read(format!(
        "{}/tests/fixtures/tls/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .expect("tls fixture")
}

#[tokio::test]
async fn tls_connect_login_with_ca_verification() {
    let cert = fixture("server.pem");
    let key = fixture("server-key.pem");
    let ca = fixture("ca.pem");

    let certs: Vec<_> = rustls_pemfile::certs(&mut cert.as_slice())
        .collect::<Result<_, _>>()
        .expect("certs");
    let key = rustls_pemfile::private_key(&mut key.as_slice())
        .expect("key parse")
        .expect("private key");
    let config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .expect("server config");
    let acceptor = TlsAcceptor::from(Arc::new(config));

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.expect("accept");
        let mut tls = acceptor.accept(socket).await.expect("tls accept");
        let mut buffer = vec![0u8; 256];
        let read = tls.read(&mut buffer).await.expect("read");
        let login = Jt809Frame::decode(&buffer[..read]).expect("login frame");
        assert_eq!(login.msg_id, jt809::MSG_LOGIN_REQUEST);
        let response = Jt809Frame {
            msg_id: MSG_LOGIN_RESPONSE,
            msg_sn: login.msg_sn,
            encrypt: 0,
            compress: 0,
            body: vec![0],
        };
        tls.write_all(&response.encode()).await.expect("response");
    });

    let login = Jt809Login {
        user_id: 1,
        password: "secret".to_string(),
        center_id: 1,
        down_link_ip: "127.0.0.1".to_string(),
        down_link_port: 9001,
    };
    let mut connection =
        jt809::Jt809Connection::connect_tls(&addr.to_string(), "localhost", &ca, 4)
            .await
            .expect("tls connect");
    connection.send_login(&login).await.expect("send login");
    let inbound = connection.receive().await.expect("receive");
    assert!(matches!(inbound, jt809::Jt809Inbound::LoginResponse(_)));
    assert_eq!(connection.link().state(), Jt809SessionState::Online);
    server.await.expect("server");
}
