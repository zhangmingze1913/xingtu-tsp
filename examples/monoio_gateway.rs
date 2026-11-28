//! Optional Monoio JT/T 808 framing probe.
//!
//! This is an opt-in runtime example, not the production Axum/SQLx gateway.
//! Monoio is thread-per-core and currently supports Linux/macOS only.
//!
//! Build on Linux:
//!   cargo run --example monoio_gateway --features monoio-runtime

#[cfg(target_os = "linux")]
mod linux {
    use monoio::io::{AsyncReadRent, AsyncWriteRentExt};
    use monoio::net::{TcpListener, TcpStream};
    use xingtu_tsp::jt808::{build_frame, parse_frame, MSG_GENERAL_RESPONSE};

    pub async fn run() -> std::io::Result<()> {
        let listener = TcpListener::bind("127.0.0.1:18081")?;
        println!("monoio JT808 framing probe listening on 127.0.0.1:18081");
        loop {
            let (stream, peer) = listener.accept().await?;
            println!("accepted {peer}");
            monoio::spawn(handle(stream));
        }
    }

    async fn handle(mut stream: TcpStream) -> std::io::Result<()> {
        let mut buffer: Vec<u8> = Vec::with_capacity(2048);
        loop {
            let (result, returned) = stream.read(buffer).await;
            let read = result?;
            buffer = returned;
            if read == 0 {
                return Ok(());
            }
            process_frames(&mut stream, &mut buffer).await?;
        }
    }

    async fn process_frames(stream: &mut TcpStream, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        let mut consumed = 0usize;
        while let Some(relative_start) = buffer[consumed..].iter().position(|byte| *byte == 0x7e) {
            let start = consumed + relative_start;
            let Some(relative_end) = buffer[start + 1..].iter().position(|byte| *byte == 0x7e)
            else {
                break;
            };
            let end = start + 1 + relative_end;
            if end > start + 1 {
                if let Ok(frame) = parse_frame(&buffer[start..=end]) {
                    let ack = [(frame.serial >> 8) as u8, frame.serial as u8, 0];
                    let response =
                        build_frame(MSG_GENERAL_RESPONSE, &frame.terminal_id, frame.serial, &ack)
                            .map_err(|error| std::io::Error::other(format!("build ack: {error}")))?;
                    let (result, _) = stream.write_all(response).await;
                    result?;
                }
            }
            consumed = end + 1;
        }
        if consumed > 0 {
            buffer.drain(..consumed);
        }
        Ok(())
    }
}

#[cfg(target_os = "linux")]
#[monoio::main]
async fn main() -> std::io::Result<()> {
    linux::run().await
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("monoio runtime probe is supported on Linux/macOS only");
}
