use std::io;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

pub struct ThalesClient {
    pub host: String,
}

impl ThalesClient {
    pub async fn translate_pin(
        &self,
        src_key: &str,
        dst_key: &str,
        pan: &str,
        pin_block: &str,
    ) -> Result<String, io::Error> {
        let mut socket = TcpStream::connect(&self.host).await?;

        // Format CA + src(16) + dst(16) + len(2) + pan(12) + pin(16)
        let mut command = String::new();
        command.push_str("CA");
        command.push_str(src_key);
        command.push_str(dst_key);
        command.push_str("12");
        
        let formatted_pan = if pan.len() >= 12 {
            &pan[pan.len() - 13..pan.len() - 1]
        } else {
            pan
        };
        
        command.push_str(formatted_pan);
        command.push_str(pin_block);

        let len = command.len() as u16;
        let mut payload = len.to_be_bytes().to_vec();
        payload.extend_from_slice(command.as_bytes());

        // Dispatch CA query
        socket.write_all(&payload).await?;

        // Await CB mapped response
        let mut len_buf = [0u8; 2];
        socket.read_exact(&mut len_buf).await?;
        let resp_len = u16::from_be_bytes(len_buf) as usize;

        let mut resp_buf = vec![0u8; resp_len];
        socket.read_exact(&mut resp_buf).await?;

        let response = String::from_utf8_lossy(&resp_buf);
        if response.starts_with("CB") {
            let translated = response[2..].to_string(); 
            Ok(translated)
        } else {
            Err(io::Error::new(io::ErrorKind::Other, "Invalid HSM Response"))
        }
    }
}
