use tokio::net::TcpStream;
use tokio::io::{AsyncWriteExt, AsyncReadExt};
use std::time::Duration;

#[tokio::main]
async fn main() {
    for i in 0..10 {
        let mut stream = TcpStream::connect("127.0.0.1:8000").await.unwrap();
        stream.set_nodelay(true).unwrap();

        // 0200 + 00..00 + 12 chars RRN 
        let rrn = format!("F82F9E0ABCD{}", i);
        let mut iso_bytes = bytes::BytesMut::new();
        iso_bytes.extend_from_slice(b"0200");
        iso_bytes.extend_from_slice(&[0x00; 8]);
        iso_bytes.extend_from_slice(rrn.as_bytes());

        let len_bytes = (iso_bytes.len() as u16).to_be_bytes();
        let mut payload = Vec::with_capacity(2 + iso_bytes.len());
        payload.extend_from_slice(&len_bytes);
        payload.extend_from_slice(&iso_bytes);

        println!("Sending payload {}...", i);
        let start = std::time::Instant::now();
        if stream.write_all(&payload).await.is_ok() {
            let mut head = [0u8; 2];
            match tokio::time::timeout(Duration::from_millis(20000), stream.read_exact(&mut head)).await {
                Ok(Ok(_)) => {
                    let len = u16::from_be_bytes(head) as usize;
                    let mut body = vec![0u8; len];
                    if stream.read_exact(&mut body).await.is_ok() {
                        println!("Received body for {} in {:?}ms", i, start.elapsed().as_millis());
                    }
                }
                Ok(Err(e)) => println!("Read error: {}", e),
                Err(_) => println!("Timeout waiting for response!"),
            }
        }
    }
}
