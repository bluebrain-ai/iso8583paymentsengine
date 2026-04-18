use axum::{routing::post, Router};
use tokio::net::TcpStream;
use tokio::io::AsyncWriteExt;
use iso_dialect::{IsoDialect, Base24Dialect};
use payment_proto::canonical::{UniversalPaymentEvent, MessageClass, TransactionType, ProcessingCode, Stan, Rrn, ResponseCode, LocalTime, LocalDate};
use bytes::Bytes;
use tower_http::cors::{Any, CorsLayer};

#[tokio::main]
async fn main() {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);
        
    let app = Router::new()
        .route("/api/trigger/apple-pay", post(trigger_apple_pay))
        .layer(cors);
        
    let listener = tokio::net::TcpListener::bind("0.0.0.0:3002").await.unwrap();
    println!("Mock ATM Token Generator listening on 0.0.0.0:3002");
    axum::serve(listener, app).await.unwrap();
}

async fn trigger_apple_pay() -> &'static str {
    let tx = UniversalPaymentEvent {
        message_class: MessageClass::Authorization,
        transaction_type: TransactionType::Purchase,
        mti: Bytes::from_static(b"0100"),
        // DPAN placed in FPAN because during encoding, the dialect injects `tx.fpan` into Field 2
        fpan: Bytes::from("4000000000009999".as_bytes()),
        dpan: None,
        is_tokenized: true,
        tavv_cryptogram: Some("AABBCCDDEEFF0011".to_string()),
        processing_code: ProcessingCode("000000".to_string()),
        amount: 8500, // $85.00 Tap
        stan: Stan("654321".to_string()),
        local_time: LocalTime("140230".to_string()),
        local_date: LocalDate("0414".to_string()),
        rrn: Rrn("000123456789".to_string()),
        response_code: ResponseCode("".to_string()),
        acquirer_id: Bytes::from_static(b"400123"),
        pin_block: Bytes::new(),
        risk_score: 0,
    };
    
    let dialect = Base24Dialect;
    if let Ok(encoded) = dialect.encode(&tx) {
        let length = encoded.len() as u16;
        let mut buf = length.to_be_bytes().to_vec();
        buf.extend_from_slice(&encoded);
        
        if let Ok(mut stream) = TcpStream::connect("127.0.0.1:8000").await {
            if stream.write_all(&buf).await.is_ok() {
                return "Successfully Trigerred Apple Pay Token Flow";
            }
        }
    }
    
    "Failed to send Apple Pay Token"
}
