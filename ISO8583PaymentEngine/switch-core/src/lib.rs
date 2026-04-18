pub mod context;
pub mod routing;
pub mod engine;
pub mod journal;
pub mod telemetry;
pub mod state;
pub mod hsm_client;
pub mod tsp_vault;

use payment_proto::Transaction;

pub fn to_pb(canonical: &payment_proto::canonical::UniversalPaymentEvent) -> payment_proto::Transaction {
    payment_proto::Transaction {
        mti: String::from_utf8_lossy(&canonical.mti).to_string(),
        pan: String::from_utf8_lossy(&canonical.fpan).to_string(),
        processing_code: canonical.processing_code.0.clone(),
        amount: canonical.amount as i64,
        stan: canonical.stan.0.clone(),
        rrn: canonical.rrn.0.clone(),
        response_code: canonical.response_code.0.clone(),
        acquirer_id: String::from_utf8_lossy(&canonical.acquirer_id).to_string(),
        pin_block: String::from_utf8_lossy(&canonical.pin_block).to_string(),
        local_time: canonical.local_time.0.clone(),
        local_date: canonical.local_date.0.clone(),
    }
}

pub fn from_pb(pb: &payment_proto::Transaction) -> payment_proto::canonical::UniversalPaymentEvent {
    payment_proto::canonical::UniversalPaymentEvent {
        message_class: match pb.mti.as_str() {
            "0100" | "0110" => payment_proto::canonical::MessageClass::Authorization,
            "0200" | "0210" => payment_proto::canonical::MessageClass::Financial,
            "0420" | "0421" => payment_proto::canonical::MessageClass::ReversalAdvice,
            "0430" => payment_proto::canonical::MessageClass::ReversalResponse,
            "0800" => payment_proto::canonical::MessageClass::NetworkManagement,
            "0810" => payment_proto::canonical::MessageClass::NetworkManagementResponse,
            _ => payment_proto::canonical::MessageClass::Financial,
        },
        transaction_type: payment_proto::canonical::TransactionType::Purchase,
        mti: bytes::Bytes::copy_from_slice(pb.mti.as_bytes()),
        fpan: bytes::Bytes::copy_from_slice(pb.pan.as_bytes()),
        dpan: None,
        is_tokenized: false,
        tavv_cryptogram: None,
        processing_code: payment_proto::canonical::ProcessingCode(pb.processing_code.clone()),
        amount: pb.amount as u64,
        stan: payment_proto::canonical::Stan(pb.stan.clone()),
        local_time: payment_proto::canonical::LocalTime(pb.local_time.clone()),
        local_date: payment_proto::canonical::LocalDate(pb.local_date.clone()),
        rrn: payment_proto::canonical::Rrn(pb.rrn.clone()),
        response_code: payment_proto::canonical::ResponseCode(pb.response_code.clone()),
        acquirer_id: bytes::Bytes::copy_from_slice(pb.acquirer_id.as_bytes()),
        pin_block: bytes::Bytes::copy_from_slice(pb.pin_block.as_bytes()),
        risk_score: 0, // proto Transaction has no risk_score; safe default
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use journal::Journaler;
    use engine::SwitchEngine;
    use std::thread;

    #[test]
    fn test_seda_pipeline_with_journaler() {
        let temp_dir = std::env::temp_dir();
        let log_path = temp_dir.join("test_wal.log");
        let size: usize = 10 * 1024 * 1024; // 10MB Mmap alloc
        let journaler = Journaler::new(&log_path, size).expect("Failed to initialize journaler");
        let telemetry = telemetry::TelemetryLogger::new();
        let engine = SwitchEngine::start(journaler, telemetry, 15_000);

        let ingress = engine.ingress();
        let egress = engine.visa_egress();

        let producer = thread::spawn(move || {
            for i in 0..1000 {
                let tx = crate::from_pb(&Transaction {
                    mti: "0100".to_string(),
                    pan: "4111111111111111".to_string(),
                    processing_code: "000000".to_string(),
                    amount: 1000,
                    stan: format!("{:06}", i),
                    rrn: "123456789012".to_string(),
                    response_code: "00".to_string(),
                    acquirer_id: "999999".to_string(),
                    pin_block: "".to_string(),
                    local_time: "120000".to_string(),
                    local_date: "1231".to_string(),
                });
                let ctx = crate::context::TransactionContext {
                    transaction: tx,
                    responder: None,
                };
                ingress.send(ctx).unwrap();
            }
        });

        let mut count = 0;
        // The loop acts as the egress consumer
        for _ in 0..1000 {
            if let Ok(_) = egress.recv() {
                count += 1;
            }
        }

        producer.join().unwrap();
        assert_eq!(count, 1000, "Should have properly journalled and routed all 1000 transactions");

        // Cleanup
        let _ = std::fs::remove_file(log_path);
    }

    #[test]
    fn test_engine_idempotency_guard() {
        let temp_dir = std::env::temp_dir();
        let log_path = temp_dir.join("test_wal_idempotency.log");
        let size: usize = 10 * 1024 * 1024;
        let journaler = Journaler::new(&log_path, size).expect("Failed to initialize journaler");
        let telemetry = telemetry::TelemetryLogger::new();
        let engine = SwitchEngine::start(journaler, telemetry, 15_000);

        let ingress = engine.ingress();
        let egress = engine.visa_egress();

        let tx1 = crate::from_pb(&Transaction {
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
        let tx2 = tx1.clone(); // Exactly the same

        let producer = thread::spawn(move || {
            let cx1 = crate::context::TransactionContext { transaction: tx1, responder: None };
            let cx2 = crate::context::TransactionContext { transaction: tx2, responder: None };
            ingress.send(cx1).unwrap();
            ingress.send(cx2).unwrap();
        });

        let mut count = 0;
        // The first transaction should be routed
        if let Ok(_) = egress.recv_timeout(std::time::Duration::from_secs(1)) {
            count += 1;
        }

        // The second one should be dropped, so it should timeout
        let result = egress.recv_timeout(std::time::Duration::from_millis(500));
        assert!(result.is_err(), "Duplicate transaction was routed instead of dropped!");

        producer.join().unwrap();
        assert_eq!(count, 1, "Exactly one transaction should pass the idempotency guard");
        
        let _ = std::fs::remove_file(log_path);
    }
    #[test]
    fn test_late_bank_reply_dropped() {
        let temp_dir = std::env::temp_dir();
        let log_path = temp_dir.join("test_late_reply.wal");
        let size: usize = 10 * 1024 * 1024;
        let journaler = Journaler::new(&log_path, size).unwrap();
        let telemetry = telemetry::TelemetryLogger::new();
        
        let engine = SwitchEngine::start(journaler, telemetry, 100);

        let ingress = engine.ingress();
        let bank_reply = engine.bank_reply();

        let tx = crate::from_pb(&Transaction {
            mti: "0100".to_string(),
            pan: "4111111111111111".to_string(),
            processing_code: "000000".to_string(),
            amount: 1000,
            stan: "888888".to_string(),
            rrn: "LATE_REPLY_123".to_string(),
            response_code: "00".to_string(),
            acquirer_id: "AcquirerA".to_string(),
            pin_block: "".to_string(),
            local_time: "120000".to_string(),
            local_date: "1231".to_string(),
        });

        let ctx = crate::context::TransactionContext {
            transaction: tx.clone(),
            responder: None,
        };

        ingress.send(ctx).unwrap();

        thread::sleep(std::time::Duration::from_millis(250));

        let mut approval_tx = crate::to_pb(&tx);
        approval_tx.mti = "0110".to_string(); 

        bank_reply.send(approval_tx).unwrap();

        thread::sleep(std::time::Duration::from_millis(100));

        let visa_egress = engine.visa_egress();
        let mut count = 0;
        let mut found_reversal = false;
        
        while let Ok(msg) = visa_egress.try_recv() {
            count += 1;
            if msg.transaction.mti.as_ref() == b"0420" {
                found_reversal = true;
            }
        }
        
        assert_eq!(count, 2, "Original 0100 and Reversal 0420 should have routed");
        assert!(found_reversal, "Engine failed to build and forward the Auto-Reversal target!");
        
        let _ = std::fs::remove_file(log_path);
    }
    #[test]
    fn test_stip_offline_fallback() {
        let temp_dir = std::env::temp_dir();
        let log_path = temp_dir.join("test_stip_fallback.wal");
        let size: usize = 10 * 1024 * 1024;
        let journaler = Journaler::new(&log_path, size).unwrap();
        let telemetry = telemetry::TelemetryLogger::new();
        
        let engine = SwitchEngine::start(journaler, telemetry, 100);

        let ingress = engine.ingress();
        
        // Let's seed STIP database directly.
        let _ = engine.stip_db.clear(); // Reset 

        let profile = crate::context::CardProfile {
            balance: 50000, // 500.00
            is_blocked: false,
        };
        let encoded = bincode::serialize(&profile).unwrap();
        engine.stip_db.insert(b"4111222233334444", encoded).unwrap();
        engine.stip_db.flush().unwrap();
        
        // We will just do a fast fill hitting exactly 10,100!
        for i in 0..10_100 {
            let tx = crate::from_pb(&Transaction {
                mti: "0100".to_string(),
                pan: "4111222233334444".to_string(),
                processing_code: "000000".to_string(),
                amount: 10000, // $100.00 
                stan: format!("{:06}", i),
                rrn: format!("STIP_TEST_{}", i),
                response_code: "00".to_string(),
                acquirer_id: "AcquirerA".to_string(),
                pin_block: "".to_string(),
                local_time: "120000".to_string(),
                local_date: "1231".to_string(),
            });

            let ctx = crate::context::TransactionContext {
                transaction: tx,
                responder: None,
            };
            
            ingress.send(ctx).unwrap();
        }

        // Wait to allow SEDA bounding and timeout executions (100ms * timeouts)
        // Accommodating huge 10k Journal log writes overhead
        thread::sleep(std::time::Duration::from_millis(5000));

        let cached = engine.stip_db.get(b"4111222233334444").unwrap().unwrap();
        let decoded: crate::context::CardProfile = bincode::deserialize(&cached).unwrap();
        
        assert!(decoded.balance < 50000, "STIP must have been executed & decremented natively!");

        let _ = std::fs::remove_file(log_path);
        let _ = engine.stip_db.clear();
    }
}
