use tokio::net::TcpListener;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub async fn run_simulator(host: String) {
    if let Ok(listener) = TcpListener::bind(&host).await {
        while let Ok((mut socket, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut len_buf = [0u8; 2];
                if socket.read_exact(&mut len_buf).await.is_ok() {
                    let len = u16::from_be_bytes(len_buf) as usize;
                    let mut buf = vec![0u8; len];
                    if socket.read_exact(&mut buf).await.is_ok() {
                        let cmd = String::from_utf8_lossy(&buf);
                        if cmd.starts_with("CA") {
                            // Yield successful CB translation dummy
                            let resp = "CB9999999999AABBCC";
                            let resp_len = resp.len() as u16;
                            let mut payload = resp_len.to_be_bytes().to_vec();
                            payload.extend_from_slice(resp.as_bytes());
                            let _ = socket.write_all(&payload).await;
                        }
                    }
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::ThalesClient;

    #[tokio::test]
    async fn test_hsm_translation() {
        let host = "127.0.0.1:0";
        let listener = TcpListener::bind(host).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let mut len_buf = [0u8; 2];
                socket.read_exact(&mut len_buf).await.unwrap();
                let len = u16::from_be_bytes(len_buf) as usize;
                let mut buf = vec![0u8; len];
                socket.read_exact(&mut buf).await.unwrap();
                
                // Return explicitly mocked response test target
                let resp = "CB8888888888FFFFFF";
                let resp_len = resp.len() as u16;
                let mut payload = resp_len.to_be_bytes().to_vec();
                payload.extend_from_slice(resp.as_bytes());
                socket.write_all(&payload).await.unwrap();
            }
        });

        let client = ThalesClient { host: format!("127.0.0.1:{}", port) };
        let translated = client.translate_pin(
            "ZPK1234567890123", 
            "AWK1234567890123", 
            "4111111111111111", 
            "1234567890ABCDEF"
        ).await.unwrap();
        
        assert_eq!(translated, "8888888888FFFFFF");
    }
}
