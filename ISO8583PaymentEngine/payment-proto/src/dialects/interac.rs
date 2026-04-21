use crate::iso_dialect::{DialectAdapter, Iso8583Message, ParseError, read_fixed, read_llvar, read_lllvar, is_bit_set, set_bit, write_fixed, write_llvar, write_lllvar};
use crate::canonical::{UniversalPaymentEvent, MessageClass, TransactionType, ProcessingCode, Stan, Rrn, ResponseCode, LocalTime, LocalDate};

pub struct InteracDialect;

impl DialectAdapter for InteracDialect {
    fn parse(&self, raw_msg: &Iso8583Message) -> Result<UniversalPaymentEvent, ParseError> {
        let raw = &raw_msg.payload;
        if raw.len() < 12 { return Err(ParseError::LengthError); }
        let mut offset = 0;
        let mti = read_fixed(raw, &mut offset, 4)?;

        let mut bitmap = [0u8; 8];
        bitmap.copy_from_slice(read_fixed(raw, &mut offset, 8)?);

        let mti_str = String::from_utf8_lossy(mti);
        let message_class = match mti_str.as_ref() {
            "0100" | "0110" => MessageClass::Authorization,
            "0200" | "0210" => MessageClass::Financial,
            "0420" | "0421" => MessageClass::ReversalAdvice,
            "0430" => MessageClass::ReversalResponse,
            "0800" => MessageClass::NetworkManagement,
            "0810" => MessageClass::NetworkManagementResponse,
            _ => MessageClass::Financial,
        };

        let mut tx = UniversalPaymentEvent {
            message_class,
            transaction_type: TransactionType::Purchase,
            mti: bytes::Bytes::copy_from_slice(mti),
            fpan: bytes::Bytes::new(),
            dpan: None,
            is_tokenized: false,
            tavv_cryptogram: None,
            processing_code: ProcessingCode(String::new()),
            amount: 0,
            stan: Stan(String::new()),
            local_time: LocalTime(String::new()),
            local_date: LocalDate(String::new()),
            rrn: Rrn(String::new()),
            response_code: ResponseCode(String::new()),
            acquirer_id: bytes::Bytes::new(),
            pin_block: bytes::Bytes::new(),
            risk_score: 0,
            requires_instant_clearing: mti_str == "0200", 
            domestic_settlement_data: None,
            source_account: None,
            destination_account: None,
            original_data_elements: None,
            mac_data: None,
            is_reversal: false,
        };

        for field in 1..=64 {
            if is_bit_set(&bitmap, field) {
                match field {
                    2 => tx.fpan = bytes::Bytes::copy_from_slice(read_llvar(raw, &mut offset)?),
                    3 => tx.processing_code = ProcessingCode(String::from_utf8_lossy(read_fixed(raw, &mut offset, 6)?).to_string()),
                    4 => {
                        let amt_str = String::from_utf8_lossy(read_fixed(raw, &mut offset, 12)?);
                        tx.amount = amt_str.parse().unwrap_or(0);
                    },
                    11 => tx.stan = Stan(String::from_utf8_lossy(read_fixed(raw, &mut offset, 6)?).to_string()),
                    12 => tx.local_time = LocalTime(String::from_utf8_lossy(read_fixed(raw, &mut offset, 6)?).to_string()),
                    13 => tx.local_date = LocalDate(String::from_utf8_lossy(read_fixed(raw, &mut offset, 4)?).to_string()),
                    22 => {
                        let entry_mode = String::from_utf8_lossy(read_fixed(raw, &mut offset, 3)?).to_string();
                        if entry_mode == "071" || entry_mode == "072" {
                            tx.is_tokenized = true;
                        }
                    },
                    32 => tx.acquirer_id = bytes::Bytes::copy_from_slice(read_llvar(raw, &mut offset)?),
                    37 => tx.rrn = Rrn(String::from_utf8_lossy(read_fixed(raw, &mut offset, 12)?).to_string()),
                    39 => tx.response_code = ResponseCode(String::from_utf8_lossy(read_fixed(raw, &mut offset, 2)?).to_string()),
                    48 => {
                        let f48 = String::from_utf8_lossy(read_lllvar(raw, &mut offset)?).to_string();
                        // For Interac, extraction of Surcharge and PIN offset from TLV.
                        // Assuming simple extraction logic based on "SUR:" and "PIN:" prefixes if generic.
                        // We will store it in the tavv cryptogram for now, or just extract them logically.
                        tx.tavv_cryptogram = Some(f48); 
                    },
                    52 => tx.pin_block = bytes::Bytes::copy_from_slice(read_fixed(raw, &mut offset, 16)?),
                    63 => {
                        let f63 = String::from_utf8_lossy(read_lllvar(raw, &mut offset)?).to_string();
                        tx.domestic_settlement_data = Some(f63);
                    },
                    _ => { } 
                }
            }
        }
        
        Ok(tx)
    }

    fn build_response(&self, event: &UniversalPaymentEvent) -> Iso8583Message {
        let mut data = bytes::BytesMut::new();
        let mut bitmap = [0u8; 8];
        let mut payload = bytes::BytesMut::new();

        if !event.fpan.is_empty() {
            set_bit(&mut bitmap, 2);
            write_llvar(&mut payload, &event.fpan);
        }
        
        if !event.processing_code.0.is_empty() {
            set_bit(&mut bitmap, 3);
            write_fixed(&mut payload, event.processing_code.0.as_bytes(), 6, b'0', false);
        }
        
        if event.amount > 0 || (event.mti.starts_with(b"02") && event.message_class == crate::canonical::MessageClass::Financial) {
            set_bit(&mut bitmap, 4);
            let amt_str = format!("{:012}", event.amount);
            write_fixed(&mut payload, amt_str.as_bytes(), 12, b'0', false);
        }

        if !event.stan.0.is_empty() {
            set_bit(&mut bitmap, 11);
            write_fixed(&mut payload, event.stan.0.as_bytes(), 6, b'0', false);
        }

        if !event.local_time.0.is_empty() {
            set_bit(&mut bitmap, 12);
            write_fixed(&mut payload, event.local_time.0.as_bytes(), 6, b'0', false);
        }

        if !event.local_date.0.is_empty() {
            set_bit(&mut bitmap, 13);
            write_fixed(&mut payload, event.local_date.0.as_bytes(), 4, b'0', false);
        }

        if event.is_tokenized {
            set_bit(&mut bitmap, 22);
            write_fixed(&mut payload, b"071", 3, b'0', false);
        }

        if !event.acquirer_id.is_empty() {
            set_bit(&mut bitmap, 32);
            write_llvar(&mut payload, &event.acquirer_id);
        }

        if !event.rrn.0.is_empty() {
            set_bit(&mut bitmap, 37);
            write_fixed(&mut payload, event.rrn.0.as_bytes(), 12, b' ', true);
        }

        if !event.response_code.0.is_empty() {
            set_bit(&mut bitmap, 39);
            write_fixed(&mut payload, event.response_code.0.as_bytes(), 2, b' ', false);
        }

        if let Some(ref cryp) = event.tavv_cryptogram {
            set_bit(&mut bitmap, 48);
            write_lllvar(&mut payload, cryp.as_bytes());
        }

        if !event.pin_block.is_empty() {
            set_bit(&mut bitmap, 52);
            write_fixed(&mut payload, &event.pin_block, 16, b'0', false);
        }
        
        if let Some(ref net_data) = event.domestic_settlement_data {
            set_bit(&mut bitmap, 63);
            write_lllvar(&mut payload, net_data.as_bytes());
        }

        if event.mti.len() >= 4 {
            data.extend_from_slice(&event.mti[0..4]);
        } else {
            data.extend_from_slice(b"0110");
        }
        
        data.extend_from_slice(&bitmap);
        data.extend_from_slice(&payload);
        
        Iso8583Message { payload: data.freeze() }
    }
}
