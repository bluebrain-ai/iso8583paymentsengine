pub mod codec;
pub mod server;

#[cfg(test)]
mod tests {
    use super::*;
    use crossbeam::channel::bounded;
    use switch_core::context::TransactionContext;
    use tokio::net::{TcpListener, TcpStream};
    use tokio::io::{AsyncWriteExt, AsyncReadExt};
    use std::time::Duration;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_tcp_edge_server() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        let (core_tx, core_rx) = bounded::<TransactionContext>(100);

        tokio::spawn(async move {
            server::run_server(listener, core_tx).await;
        });

        let mut client = TcpStream::connect(format!("127.0.0.1:{}", port)).await.unwrap();
        
        let mti = b"0100";
        let bitmap = [0xFF, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
        
        let payload_len = (mti.len() + bitmap.len()) as u16;
        let mut packet = payload_len.to_be_bytes().to_vec();
        packet.extend_from_slice(mti);
        packet.extend_from_slice(&bitmap);

        client.write_all(&packet).await.unwrap();

        let mut routed_ctx = core_rx.recv_timeout(Duration::from_secs(2))
            .expect("Message failed to parse and route within timeout window");
        
        assert_eq!(routed_ctx.transaction.mti.as_ref(), b"0100");
        
        let mut simulated_reply = routed_ctx.transaction.clone();
        simulated_reply.mti = bytes::Bytes::from_static(b"0110");
        
        if let Some(responder) = routed_ctx.responder.take() {
            responder.send(simulated_reply).unwrap();
        }

        let mut length_buf = [0u8; 2];
        client.read_exact(&mut length_buf).await.unwrap();
        let length = u16::from_be_bytes(length_buf) as usize;

        let mut payload_buf = vec![0u8; length];
        client.read_exact(&mut payload_buf).await.unwrap();
        
        let returned_mti = std::str::from_utf8(&payload_buf[..4]).unwrap();
        assert_eq!(returned_mti, "0110", "Network correctly translated oneshot reply context.");
    }
}
