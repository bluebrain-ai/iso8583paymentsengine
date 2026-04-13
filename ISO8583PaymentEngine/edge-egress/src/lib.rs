pub mod adapter;
pub mod client;

#[cfg(test)]
mod tests {
    use super::*;
    use crossbeam::channel::bounded;
    use payment_proto::Transaction;
    use tokio::net::TcpListener;
    use tokio::io::AsyncReadExt;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_egress_upstream_dispatch() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        let (tx, rx) = bounded::<switch_core::context::TransactionContext>(100);

        let (bank_reply_tx, _) = crossbeam::channel::bounded(100);
        let c = client::UpstreamClient {
            host: format!("127.0.0.1:{}", port),
            rx,
            saf_timeout_ms: 10_000,
            bank_reply_tx,
            saf_db: sled::open("saf_test1.db").unwrap(),
        };

        tokio::spawn(async move {
            c.run().await;
        });

        let mock_tx = switch_core::from_pb(&Transaction {
            mti: "0100".to_string(),
            pan: "4111111111111111".to_string(),
            processing_code: "000000".to_string(),
            amount: 1000,
            stan: "123456".to_string(),
            rrn: "123456789012".to_string(),
            response_code: "00".to_string(),
            acquirer_id: "AcquirerA".to_string(),
            pin_block: "".to_string(),
            local_time: "120000".to_string(),
            local_date: "1231".to_string(),
        });

        let ctx = switch_core::context::TransactionContext {
            transaction: mock_tx,
            responder: None,
        };

        tx.send(ctx).unwrap();

        let (mut socket, _) = listener.accept().await.unwrap();
        
        let mut length_buf = [0u8; 2];
        socket.read_exact(&mut length_buf).await.unwrap();
        let length = u16::from_be_bytes(length_buf) as usize;

        assert_eq!(length, 24, "0100 + Bitmap + RRN is 24 bytes");

        let mut payload_buf = vec![0u8; length];
        socket.read_exact(&mut payload_buf).await.unwrap();
        
        let mti = std::str::from_utf8(&payload_buf[..4]).unwrap();
        assert_eq!(mti, "0100");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_saf_worker_offline_recovery() {
        let _ = std::fs::remove_dir_all("saf_test2.db"); // Clear Sled state

        let (tx, rx) = bounded::<switch_core::context::TransactionContext>(10);
        let port = 9189; // Target a hardcoded offline port
        
        let (bank_reply_tx, _) = crossbeam::channel::bounded(10);
        let c = client::UpstreamClient {
            host: format!("127.0.0.1:{}", port),
            rx,
            saf_timeout_ms: 2000, 
            bank_reply_tx,
            saf_db: sled::open("saf_test2.db").unwrap(),
        };

        tokio::spawn(async move {
            c.run().await;
        });

        // 1. Send 0420 exactly matching Offline Target
        let mock_0420 = switch_core::from_pb(&Transaction {
            mti: "0420".to_string(),
            pan: "4111111111111111".to_string(),
            processing_code: "000000".to_string(),
            amount: 1000,
            stan: "888".to_string(),
            rrn: "RRN_SAF_XXX".to_string(),
            response_code: "00".to_string(),
            acquirer_id: "Acq".to_string(),
            pin_block: "".to_string(),
            local_time: "120000".to_string(),
            local_date: "1231".to_string(),
        });

        tx.send(switch_core::context::TransactionContext {
            transaction: mock_0420,
            responder: None,
        }).unwrap();

        // 2. Wait explicitly allowing Egress internal fail-fast loops to hit `sled`.
        // The disconnected hot loop sleeps for 1 sec if empty, so wait 1200ms.
        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;

        // 3. Boot dummy Bank Listener!
        let listener = TcpListener::bind(format!("127.0.0.1:{}", port)).await.unwrap();

        // 4. SafWorker loop picks up 500ms bounds, flushes the socket.
        let (mut socket, _) = listener.accept().await.unwrap();

        let mut length_buf = [0u8; 2];
        socket.read_exact(&mut length_buf).await.unwrap();
        let length = u16::from_be_bytes(length_buf) as usize;

        let mut payload_buf = vec![0u8; length];
        socket.read_exact(&mut payload_buf).await.unwrap();
        
        // Assert 0420 is delivered correctly!
        let mti = std::str::from_utf8(&payload_buf[..4]).unwrap();
        assert_eq!(mti, "0420", "SAF flush must deliver 0420 explicitly");

        // Give sled a tick to complete remove() task mappings
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        
        let _ = std::fs::remove_dir_all("saf_test2.db"); 
    }
}
