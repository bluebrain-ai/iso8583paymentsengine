use crate::iso_dialect::{DialectAdapter, Iso8583Message, ParseError, read_fixed, read_llvar, write_fixed, write_llvar, is_bit_set, set_bit, write_lllvar, read_lllvar};
use crate::canonical::{UniversalPaymentEvent, MessageClass, TransactionType, ProcessingCode, Stan, Rrn, ResponseCode, LocalTime, LocalDate};

pub struct UkFpsDialect;

impl DialectAdapter for UkFpsDialect {
    fn parse(&self, raw_msg: &Iso8583Message) -> Result<UniversalPaymentEvent, ParseError> {
        let raw = &raw_msg.payload;
        if raw.len() < 12 { return Err(ParseError::LengthError); }
        let mut offset = 0;
        let mti = read_fixed(raw, &mut offset, 4)?;

        // UK FPS uses secondary bitmaps to reach fields 102/103. 
        // For simplicity in this mock protocol adapter, we'll scan standard F64+ and secondary bitsets.
        let mut bitmap = [0u8; 8];
        bitmap.copy_from_slice(read_fixed(raw, &mut offset, 8)?);
        
        let mut has_secondary_bitmap = is_bit_set(&bitmap, 1);
        let mut secondary_bitmap = [0u8; 8];
        if has_secondary_bitmap {
            secondary_bitmap.copy_from_slice(read_fixed(raw, &mut offset, 8)?);
        }

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
            requires_instant_clearing: false,
            domestic_settlement_data: None,
            source_account: None,
            destination_account: None,
            original_data_elements: None,
            mac_data: None,
            is_reversal: false,
        };

        for field in 1..=128 {
            let is_set = if field <= 64 {
                is_bit_set(&bitmap, field)
            } else if has_secondary_bitmap {
                is_bit_set(&secondary_bitmap, field - 64)
            } else {
                false
            };

            if is_set {
                match field {
                    1 => {}, // Secondary bitmap presence indicator, already consumed
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
                    52 => tx.pin_block = bytes::Bytes::copy_from_slice(read_fixed(raw, &mut offset, 16)?),
                    102 => {
                        let f102 = String::from_utf8_lossy(read_llvar(raw, &mut offset)?).to_string();
                        tx.source_account = Some(f102.clone());
                        // If simulator hasn't provided field 2, fallback to UK FPS generic routing
                        if tx.fpan.is_empty() {
                            tx.fpan = bytes::Bytes::from(format!("UKFPS-{}", f102));
                        }
                    },
                    103 => {
                        let f103 = String::from_utf8_lossy(read_llvar(raw, &mut offset)?).to_string();
                        tx.destination_account = Some(f103);
                    },
                    _ => { 
                        // If it's another LLVAR / LLLVAR field it would break offset, but in our mocked setup we assume payload only contains required fields.
                    } 
                }
            }
        }
        
        Ok(tx)
    }

    fn build_response(&self, event: &UniversalPaymentEvent) -> Iso8583Message {
        let mut data = bytes::BytesMut::new();
        let mut bitmap = [0u8; 8];
        let mut secondary_bitmap = [0u8; 8];
        let mut payload = bytes::BytesMut::new();

        // UK FPS ignores field 2 for response as well typically. 
        
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

        if !event.pin_block.is_empty() {
            set_bit(&mut bitmap, 52);
            write_fixed(&mut payload, &event.pin_block, 16, b'0', false);
        }

        if let Some(ref acc) = event.source_account {
            set_bit(&mut bitmap, 1);
            set_bit(&mut secondary_bitmap, 102 - 64);
            write_llvar(&mut payload, acc.as_bytes());
        }

        if let Some(ref acc) = event.destination_account {
            set_bit(&mut bitmap, 1);
            set_bit(&mut secondary_bitmap, 103 - 64);
            write_llvar(&mut payload, acc.as_bytes());
        }

        if event.mti.len() >= 4 {
            data.extend_from_slice(&event.mti[0..4]);
        } else {
            data.extend_from_slice(b"0110");
        }
        
        data.extend_from_slice(&bitmap);
        if is_bit_set(&bitmap, 1) {
            data.extend_from_slice(&secondary_bitmap);
        }
        data.extend_from_slice(&payload);
        
        Iso8583Message { payload: data.freeze() }
    }
}
