//! JT/T 1078 media UDP listener. Terminals can send 1078 media packets here;
//! decoded packets are pushed into the in-process [`MediaHub`].

use std::io;

use tokio::net::UdpSocket;

use crate::standards::jt1078::Jt1078Packet;

pub async fn bind(addr: &str) -> io::Result<(UdpSocket, String)> {
    let socket = UdpSocket::bind(addr).await?;
    let local = socket.local_addr()?.to_string();
    Ok((socket, local))
}

/// Receive loop; runs until the socket errors.
pub async fn serve_socket(socket: UdpSocket) {
    let mut buffer = vec![0u8; 65536];
    loop {
        match socket.recv_from(&mut buffer).await {
            Ok((len, peer)) => match Jt1078Packet::decode(&buffer[..len]) {
                Ok(packet) => {
                    crate::media::hub::global().ingest(&packet);
                }
                Err(error) => {
                    tracing::debug!(%error, %peer, "discarded invalid JT1078 datagram");
                }
            },
            Err(error) => {
                tracing::warn!(%error, "JT1078 UDP receive failed");
                return;
            }
        }
    }
}

pub async fn serve(addr: &str) -> io::Result<()> {
    let (socket, local) = bind(addr).await?;
    tracing::info!(addr = %local, "JT/T 1078 media UDP listener ready");
    serve_socket(socket).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn packet(sim: [u8; 6]) -> Jt1078Packet {
        Jt1078Packet {
            flags: 0,
            marker_payload: 0x80,
            sequence: 1,
            sim,
            channel: 1,
            data_type: 0,
            timestamp: 1000,
            last_i_frame_interval: 0,
            last_frame_interval: 0,
            body: vec![0, 0, 0, 1, 0x65, 0x88, 0x84],
        }
    }

    #[tokio::test]
    async fn udp_packets_reach_the_media_hub() {
        let (socket, addr) = bind("127.0.0.1:0").await.expect("bind");
        tokio::spawn(serve_socket(socket));
        let client = UdpSocket::bind("127.0.0.1:0").await.expect("client");
        let sim = [0x13, 0x80, 0x00, 0x00, 0x00, 0x99];
        let raw = packet(sim).encode().expect("encode");
        client.send_to(&raw, &addr).await.expect("send");

        let key = crate::media::hub::hex_sim(&sim);
        for _ in 0..50 {
            if crate::media::hub::global().snapshot(&key, 1).is_some() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("media hub did not receive the datagram");
    }
}
