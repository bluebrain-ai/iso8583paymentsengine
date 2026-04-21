use iso_dialect::{Base24Dialect, DialectRouter};
use payment_proto::canonical::*;
use bytes::Bytes;

fn main() {
    let mut tx = UniversalPaymentEvent {
        message_class: MessageClass::Financial,
        transaction_type: TransactionType::Purchase,
        mti: Bytes::from_static(b"0200"),
        fpan: Bytes::from("4111111111111111"),
        dpan: None,
        is_tokenized: false,
        tavv_cryptogram: None,
        processing_code: ProcessingCode("000000".to_string()),
        amount: 14312,
        stan: Stan("F82300".to_string()),
        local_time: LocalTime(String::new()),
        local_date: LocalDate(String::new()),
        rrn: Rrn("F82300C842F4".to_string()),
        response_code: ResponseCode(String::new()),
        acquirer_id: bytes::Bytes::new(),
        pin_block: bytes::Bytes::new(),
        risk_score: 0,
        requires_instant_clearing: false,
        domestic_settlement_data: None,
        source_account: None,
        destination_account: None,
        original_data_elements: None,
        mac_data: None,
        is_reversal: false,
    };

    let dialect = DialectRouter::Base24(iso_dialect::Base24Dialect);
    
    println!("Encoding edge-egress format...");
    let encoded = dialect.encode(&tx).unwrap();
    println!("Encoded Hex: {}", encoded.iter().map(|b| format!("{:02x}", b)).collect::<String>());

    println!("Attempting mock-bank-node decode...");
    match dialect.decode(&encoded) {
        Ok(res) => println!("Success! {:?}", res.mti),
        Err(e) => println!("DECODE FAILED: {:?}", e),
    }
}
