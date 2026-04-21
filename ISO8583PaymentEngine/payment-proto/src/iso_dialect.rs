use crate::canonical::{UniversalPaymentEvent, MessageClass, TransactionType, ProcessingCode, Stan, LocalTime, LocalDate, Rrn, ResponseCode};
use std::fmt;

#[derive(Debug, Clone)]
pub struct Iso8583Message {
    pub payload: bytes::Bytes,
}

#[derive(Debug)]
pub enum ParseError {
    InvalidFormat,
    UnknownMTI,
    LengthError,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::InvalidFormat => write!(f, "Invalid Format"),
            ParseError::UnknownMTI => write!(f, "Unknown MTI"),
            ParseError::LengthError => write!(f, "Length Error Parsing Data"),
        }
    }
}

impl std::error::Error for ParseError {}

pub trait DialectAdapter: Send + Sync {
    fn parse(&self, raw_msg: &Iso8583Message) -> Result<UniversalPaymentEvent, ParseError>;
    fn build_response(&self, event: &UniversalPaymentEvent) -> Iso8583Message;
}

pub enum FieldType {
    Fixed(usize),
    Llvar,
    Lllvar,
}

pub trait DataDictionary: Send + Sync {
    fn field_type(&self, field: usize) -> Option<FieldType>;
}

pub struct Iso1987Dictionary;
impl DataDictionary for Iso1987Dictionary {
    fn field_type(&self, field: usize) -> Option<FieldType> {
        match field {
            2 => Some(FieldType::Llvar),
            3 => Some(FieldType::Fixed(6)),
            4 => Some(FieldType::Fixed(12)),
            11 => Some(FieldType::Fixed(6)),
            12 => Some(FieldType::Fixed(6)),
            13 => Some(FieldType::Fixed(4)),
            22 => Some(FieldType::Fixed(3)),
            32 => Some(FieldType::Llvar),
            37 => Some(FieldType::Fixed(12)),
            39 => Some(FieldType::Fixed(2)),
            48 => Some(FieldType::Lllvar),
            52 => Some(FieldType::Fixed(16)),
            64 => Some(FieldType::Fixed(16)), // MAC String
            90 => Some(FieldType::Fixed(42)), // Original Data Elements
            128 => Some(FieldType::Fixed(16)), // MAC String Secondary
            _ => None,
        }
    }
}

pub struct Iso1993Dictionary;
impl DataDictionary for Iso1993Dictionary {
    fn field_type(&self, field: usize) -> Option<FieldType> {
        Iso1987Dictionary.field_type(field) // Delegate generic for now
    }
}

pub struct Iso2003Dictionary;
impl DataDictionary for Iso2003Dictionary {
    fn field_type(&self, field: usize) -> Option<FieldType> {
        Iso1987Dictionary.field_type(field) // Delegate generic for now
    }
}

pub struct StandardDialect;

impl DialectAdapter for StandardDialect {
    fn parse(&self, raw_msg: &Iso8583Message) -> Result<UniversalPaymentEvent, ParseError> {
        let raw = &raw_msg.payload;
        if raw.len() < 12 { return Err(ParseError::LengthError); }
        let mut offset = 0;
        let mti = read_fixed(raw, &mut offset, 4)?;

        // Phase 1: Dynamic MTI Schema Router
        let dict: Box<dyn DataDictionary> = match mti[0] {
            b'0' => Box::new(Iso1987Dictionary),
            b'1' => Box::new(Iso1993Dictionary),
            b'2' => Box::new(Iso2003Dictionary),
            _ => Box::new(Iso1987Dictionary), // Fallback
        };

        // Phase 2: Recursive Bitmap Traversal
        let mut bitmap = Vec::new();
        let mut read_more = true;
        let mut bit_offset = 1;
        while read_more {
            let chunk = read_fixed(raw, &mut offset, 8)?;
            bitmap.extend_from_slice(chunk);
            // Check MSB of this chunk
            if (chunk[0] & 0x80) != 0 && bit_offset < 129 {
                read_more = true;
                bit_offset += 64;
            } else {
                read_more = false;
            }
        }

        let message_class = match mti {
            b"0100" | b"0110" => MessageClass::Authorization,
            b"0200" | b"0210" => MessageClass::Financial,
            b"0420" | b"0421" => MessageClass::ReversalAdvice,
            b"0430" => MessageClass::ReversalResponse,
            b"0800" => MessageClass::NetworkManagement,
            b"0810" => MessageClass::NetworkManagementResponse,
            _ => MessageClass::Financial,
        };

        let is_reversal = match mti {
            b"0400" | b"0420" | b"1420" => true,
            _ => false,
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
            is_reversal,
        };

        let total_fields = bitmap.len() * 8;
        for field in 1..=total_fields {
            // Phase 2.3: Skip extension bits
            if field == 1 || field == 65 || field == 129 {
                continue;
            }
            if is_bit_set(&bitmap, field) {
                if let Some(ftype) = dict.field_type(field) {
                    let field_data = match ftype {
                        FieldType::Fixed(len) => read_fixed(raw, &mut offset, len)?,
                        FieldType::Llvar => read_llvar(raw, &mut offset)?,
                        FieldType::Lllvar => read_lllvar(raw, &mut offset)?,
                    };

                    match field {
                        2 => tx.fpan = bytes::Bytes::copy_from_slice(field_data),
                        3 => tx.processing_code = ProcessingCode(String::from_utf8_lossy(field_data).to_string()),
                        4 => {
                            let amt_str = String::from_utf8_lossy(field_data);
                            tx.amount = amt_str.parse().unwrap_or(0);
                        },
                        11 => tx.stan = Stan(String::from_utf8_lossy(field_data).to_string()),
                        12 => tx.local_time = LocalTime(String::from_utf8_lossy(field_data).to_string()),
                        13 => tx.local_date = LocalDate(String::from_utf8_lossy(field_data).to_string()),
                        22 => {
                            let entry_mode = String::from_utf8_lossy(field_data).to_string();
                            if entry_mode == "071" || entry_mode == "072" {
                                tx.is_tokenized = true;
                            }
                        },
                        32 => tx.acquirer_id = bytes::Bytes::copy_from_slice(field_data),
                        37 => tx.rrn = Rrn(String::from_utf8_lossy(field_data).to_string()),
                        39 => tx.response_code = ResponseCode(String::from_utf8_lossy(field_data).to_string()),
                        48 => {
                            let f48 = String::from_utf8_lossy(field_data).to_string();
                            if f48.starts_with("RSK:") && f48.len() >= 8 {
                                if let Ok(score) = f48[4..7].parse::<u8>() {
                                    tx.risk_score = score;
                                }
                                let remainder = f48[8..].to_string();
                                if tx.is_tokenized && !remainder.is_empty() {
                                    tx.tavv_cryptogram = Some(remainder);
                                }
                            } else if tx.is_tokenized {
                                tx.tavv_cryptogram = Some(f48);
                            }
                        },
                        52 => tx.pin_block = bytes::Bytes::copy_from_slice(field_data),
                        64 => {
                            if !is_bit_set(&bitmap, 1) {
                                tx.mac_data = Some(String::from_utf8_lossy(field_data).to_string());
                            }
                        },
                        90 => tx.original_data_elements = Some(String::from_utf8_lossy(field_data).to_string()),
                        128 => {
                            if is_bit_set(&bitmap, 1) {
                                tx.mac_data = Some(String::from_utf8_lossy(field_data).to_string());
                            }
                        },
                        _ => {}
                    }
                } else {
                    return Err(ParseError::InvalidFormat);
                }
            }
        }
        
        Ok(tx)
    }

    fn build_response(&self, event: &UniversalPaymentEvent) -> Iso8583Message {
        let mut data = bytes::BytesMut::new();
        let mut bitmap = vec![0u8; 8];
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
        if event.is_tokenized {
            if let Some(ref cryp) = event.tavv_cryptogram {
                set_bit(&mut bitmap, 48);
                write_lllvar(&mut payload, cryp.as_bytes());
            }
        }
        if !event.pin_block.is_empty() {
            set_bit(&mut bitmap, 52);
            write_fixed(&mut payload, &event.pin_block, 16, b'0', false);
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

// Iso Utility Functions
pub(crate) fn set_bit(bitmap: &mut [u8], field: usize) {
    if field < 1 || field > bitmap.len() * 8 { return; }
    let byte_index = (field - 1) / 8;
    let bit_index = (field - 1) % 8;
    bitmap[byte_index] |= 0x80 >> bit_index;
}

pub(crate) fn is_bit_set(bitmap: &[u8], field: usize) -> bool {
    if field < 1 || field > bitmap.len() * 8 { return false; }
    let byte_index = (field - 1) / 8;
    let bit_index = (field - 1) % 8;
    (bitmap[byte_index] & (0x80 >> bit_index)) != 0
}

pub(crate) fn read_fixed<'a>(data: &'a [u8], offset: &mut usize, len: usize) -> Result<&'a [u8], ParseError> {
    if *offset + len > data.len() {
        return Err(ParseError::LengthError);
    }
    let val = &data[*offset..*offset + len];
    *offset += len;
    Ok(val)
}

pub(crate) fn read_llvar<'a>(data: &'a [u8], offset: &mut usize) -> Result<&'a [u8], ParseError> {
    let len_str = read_fixed(data, offset, 2)?;
    let len_str = std::str::from_utf8(len_str).map_err(|_| ParseError::InvalidFormat)?;
    let len = len_str.parse::<usize>().map_err(|_| ParseError::InvalidFormat)?;
    read_fixed(data, offset, len)
}

pub(crate) fn read_lllvar<'a>(data: &'a [u8], offset: &mut usize) -> Result<&'a [u8], ParseError> {
    let len_str = read_fixed(data, offset, 3)?;
    let len_str = std::str::from_utf8(len_str).map_err(|_| ParseError::InvalidFormat)?;
    let len = len_str.parse::<usize>().map_err(|_| ParseError::InvalidFormat)?;
    read_fixed(data, offset, len)
}

pub(crate) fn write_fixed(bytes: &mut bytes::BytesMut, val: &[u8], len: usize, pad_char: u8, pad_right: bool) {
    if val.len() >= len {
        bytes.extend_from_slice(&val[0..len]);
    } else {
        let padding = len - val.len();
        if pad_right {
            bytes.extend_from_slice(val);
            bytes.extend_from_slice(&vec![pad_char; padding]);
        } else {
            bytes.extend_from_slice(&vec![pad_char; padding]);
            bytes.extend_from_slice(val);
        }
    }
}

pub(crate) fn write_llvar(bytes: &mut bytes::BytesMut, val: &[u8]) {
    let len = val.len();
    if len > 99 { return; }
    bytes.extend_from_slice(format!("{:02}", len).as_bytes());
    bytes.extend_from_slice(val);
}

pub(crate) fn write_lllvar(bytes: &mut bytes::BytesMut, val: &[u8]) {
    let len = val.len();
    if len > 999 { return; }
    bytes.extend_from_slice(format!("{:03}", len).as_bytes());
    bytes.extend_from_slice(val);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_test_msg(mti: &str, bitmaps: Vec<u8>, data: &str) -> Iso8583Message {
        let mut p = bytes::BytesMut::new();
        p.extend_from_slice(mti.as_bytes());
        p.extend_from_slice(&bitmaps);
        p.extend_from_slice(data.as_bytes());
        Iso8583Message { payload: p.freeze() }
    }

    #[test]
    fn test_parse_1987_primary_only() {
        let mut bitmap = [0u8; 8];
        set_bit(&mut bitmap, 4); // Amount
        let msg = build_test_msg("0100", bitmap.to_vec(), "000000001000");

        let dialect = StandardDialect;
        let event = dialect.parse(&msg).expect("Failed to parse");
        assert_eq!(event.amount, 1000);
        assert_eq!(event.mti, b"0100".as_ref());
    }

    #[test]
    fn test_parse_1993_secondary_bitmap_present() {
        let mut bitmap = vec![0u8; 16];
        set_bit(&mut bitmap, 1); // Secondary Bitmap present
        set_bit(&mut bitmap, 4); // Amount
        // Random field in second bitmap (e.g. 70, not used but we can set something ignored)
        // Wait, if we set field 70 and it isn't mapped, StandardDialect will return an Error! 
        // Let's set NO fields beyond 64 for simplicity, or we map field 70 to Fixed(2).
        let msg = build_test_msg("1100", bitmap.to_vec(), "000000002000");

        let dialect = StandardDialect;
        let event = dialect.parse(&msg).expect("Failed to parse");
        assert_eq!(event.amount, 2000);
        assert_eq!(event.mti, b"1100".as_ref());
    }

    #[test]
    fn test_parse_2003_tertiary_bitmap_present() {
        let mut bitmap = vec![0u8; 24];
        set_bit(&mut bitmap, 1); // Secondary Bitmap present
        set_bit(&mut bitmap, 65); // Tertiary Bitmap present
        set_bit(&mut bitmap, 4); // Amount
        let msg = build_test_msg("2100", bitmap.to_vec(), "000000003000");

        let dialect = StandardDialect;
        let event = dialect.parse(&msg).expect("Failed to parse");
        assert_eq!(event.amount, 3000);
        assert_eq!(event.mti, b"2100".as_ref());
    }
}
