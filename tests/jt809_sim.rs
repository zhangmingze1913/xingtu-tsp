//! Simulated JT/T 809 platform interop: login, alarm business, ACK, reconnect.
//! This is a TCP mock, not a real regulator/TLS certification.

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use xingtu_tsp::standards::jt809::{
    self, Jt809Ack, Jt809AlarmData, Jt809BusinessEnvelope, Jt809Frame, Jt809Login,
    Jt809SessionState, MSG_LOGIN_RESPONSE, MSG_PLATFORM_ACK, MSG_UP_DYNAMIC,
};

async fn read_frame(stream: &mut TcpStream) -> Jt809Frame {
    let mut buffer = Vec::new();
    loop {
        let mut chunk = [0u8; 512];
        let read = stream.read(&mut chunk).await.expect("read");
        assert!(read > 0, "unexpected eof");
        buffer.extend_from_slice(&chunk[..read]);
        if buffer.iter().filter(|byte| **byte == 0x5d).count() >= 1 {
            return Jt809Frame::decode(&buffer).expect("decode frame");
        }
    }
}

#[tokio::test]
async fn simulated_platform_login_alarm_ack_and_reconnect() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let server = tokio::spawn(async move {
        for round in 0..2 {
            let (mut socket, _) = listener.accept().await.expect("accept");
            let login = read_frame(&mut socket).await;
            assert_eq!(login.msg_id, jt809::MSG_LOGIN_REQUEST);
            let response = Jt809Frame {
                msg_id: MSG_LOGIN_RESPONSE,
                msg_sn: login.msg_sn,
                encrypt: 0,
                compress: 0,
                body: vec![0],
            };
            socket
                .write_all(&response.encode())
                .await
                .expect("login response");

            if round == 0 {
                let business = read_frame(&mut socket).await;
                assert_eq!(business.msg_id, MSG_UP_DYNAMIC);
                let envelope =
                    Jt809BusinessEnvelope::decode(&business.body).expect("business envelope");
                assert_eq!(envelope.sub_type, jt809::SUB_REALTIME_LOCATION);
                let alarm = Jt809AlarmData::decode(&envelope.data).expect("alarm body");
                assert_eq!(alarm.alarm_type, 9);

                let ack = Jt809Frame {
                    msg_id: MSG_PLATFORM_ACK,
                    msg_sn: 99,
                    encrypt: 0,
                    compress: 0,
                    body: Jt809Ack {
                        ack_sn: business.msg_sn,
                        result: 0,
                    }
                    .encode(),
                };
                socket.write_all(&ack.encode()).await.expect("ack");
            }
        }
    });

    let login = Jt809Login {
        user_id: 1,
        password: "secret".to_string(),
        center_id: 1,
        down_link_ip: "127.0.0.1".to_string(),
        down_link_port: 9001,
    };

    let mut connection = jt809::Jt809Connection::connect_and_login(&addr.to_string(), &login, 4)
        .await
        .expect("login");
    assert_eq!(connection.link().state(), Jt809SessionState::Online);

    let alarm = Jt809AlarmData {
        alarm_type: 9,
        lat: 31.2304,
        lon: 121.4737,
        ts: [0x26, 0x11, 0x19, 0x10, 0x11, 0x12],
        tail: vec![],
    };
    let envelope = Jt809BusinessEnvelope {
        sub_type: jt809::SUB_REALTIME_LOCATION,
        data: alarm.encode().expect("alarm encode"),
    };
    connection
        .send_frame(MSG_UP_DYNAMIC, envelope.encode())
        .await
        .expect("send business");
    let inbound = connection.receive().await.expect("receive ack");
    assert!(matches!(inbound, jt809::Jt809Inbound::Ack(_)));

    let reconnect = jt809::Jt809Connection::connect_and_login(&addr.to_string(), &login, 4)
        .await
        .expect("reconnect");
    assert_eq!(reconnect.link().state(), Jt809SessionState::Online);
    server.await.expect("server");
}
