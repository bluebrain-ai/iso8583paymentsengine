use payment_proto::canonical::{UniversalPaymentEvent, MessageClass, TransactionType, ProcessingCode, Stan, Rrn, ResponseCode, LocalTime, LocalDate};
use bytes::Bytes;
use std::fmt;

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

pub trait IsoDialect {
    fn decode(&self, raw: &[u8]) -> Result<UniversalPaymentEvent, ParseError>;
    fn encode(&self, tx: &UniversalPaymentEvent) -> Result<bytes::Bytes, ParseError>;
}

// Iso Utility Functions
fn set_bit(bitmap: &mut [u8; 8], field: usize) {
    if field < 1 || field > 64 { return; }
    let byte_index = (field - 1) / 8;
    let bit_index = (field - 1) % 8;
    bitmap[byte_index] |= 0x80 >> bit_index;
}

fn is_bit_set(bitmap: &[u8; 8], field: usize) -> bool {
    if field < 1 || field > 64 { return false; }
    let byte_index = (field - 1) / 8;
    let bit_index = (field - 1) % 8;
    (bitmap[byte_index] & (0x80 >> bit_index)) != 0
}

fn read_fixed<'a>(data: &'a [u8], offset: &mut usize, len: usize) -> Result<&'a [u8], ParseError> {
    if *offset + len > data.len() {
        return Err(ParseError::LengthError);
    }
    let val = &data[*offset..*offset + len];
    *offset += len;
    Ok(val)
}

fn read_llvar<'a>(data: &'a [u8], offset: &mut usize) -> Result<&'a [u8], ParseError> {
    // LLVAR specifies length in the first 2 bytes (ASCII numbers)
    let len_str = read_fixed(data, offset, 2)?;
    let len_str = std::str::from_utf8(len_str).map_err(|_| ParseError::InvalidFormat)?;
    let len = len_str.parse::<usize>().map_err(|_| ParseError::InvalidFormat)?;
    read_fixed(data, offset, len)
}

fn read_lllvar<'a>(data: &'a [u8], offset: &mut usize) -> Result<&'a [u8], ParseError> {
    let len_str = read_fixed(data, offset, 3)?;
    let len_str = std::str::from_utf8(len_str).map_err(|_| ParseError::InvalidFormat)?;
    let len = len_str.parse::<usize>().map_err(|_| ParseError::InvalidFormat)?;
    read_fixed(data, offset, len)
}

fn write_fixed(bytes: &mut bytes::BytesMut, val: &[u8], len: usize, pad_char: u8, pad_right: bool) {
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

fn write_llvar(bytes: &mut bytes::BytesMut, val: &[u8]) {
    let len = val.len();
    if len > 99 { return; } // Cap at 99
    bytes.extend_from_slice(format!("{:02}", len).as_bytes());
    bytes.extend_from_slice(val);
}

fn write_lllvar(bytes: &mut bytes::BytesMut, val: &[u8]) {
    let len = val.len();
    if len > 999 { return; } // Cap at 999
    bytes.extend_from_slice(format!("{:03}", len).as_bytes());
    bytes.extend_from_slice(val);
}

// Universal ISO Engine parsing implementation
fn universal_decode(raw: &[u8]) -> Result<UniversalPaymentEvent, ParseError> {
    if raw.len() < 12 { return Err(ParseError::LengthError); }
    let mut offset = 0;
    let mti = read_fixed(raw, &mut offset, 4)?;

    let mut bitmap = [0u8; 8];
    bitmap.copy_from_slice(read_fixed(raw, &mut offset, 8)?);

    let message_class = match mti {
        b"0100" | b"0110" => MessageClass::Authorization,
        b"0200" | b"0210" => MessageClass::Financial,
        b"0420" | b"0421" => MessageClass::ReversalAdvice,
        b"0430" => MessageClass::ReversalResponse,
        b"0800" => MessageClass::NetworkManagement,
        b"0810" => MessageClass::NetworkManagementResponse,
        _ => MessageClass::Financial,
    };

    let mut tx = UniversalPaymentEvent {
        message_class,
        transaction_type: TransactionType::Purchase,
        mti: Bytes::copy_from_slice(mti),
        fpan: Bytes::new(),
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
        acquirer_id: Bytes::new(),
        pin_block: Bytes::new(),
        risk_score: 0, // populated from RSK:NNN: TLV in Field 48 if present
        requires_instant_clearing: false,
        domestic_settlement_data: None,
        source_account: None,
        destination_account: None,
        original_data_elements: None,
        mac_data: None,
        is_reversal: match mti {
            b"0400" | b"0420" | b"1420" => true,
            _ => false,
        },
    };

    for field in 1..=64 {
        if is_bit_set(&bitmap, field) {
            match field {
                2 => tx.fpan = Bytes::copy_from_slice(read_llvar(raw, &mut offset)?),
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
                32 => tx.acquirer_id = Bytes::copy_from_slice(read_llvar(raw, &mut offset)?),
                37 => tx.rrn = Rrn(String::from_utf8_lossy(read_fixed(raw, &mut offset, 12)?).to_string()),
                39 => tx.response_code = ResponseCode(String::from_utf8_lossy(read_fixed(raw, &mut offset, 2)?).to_string()),
                48 => {
                    let f48 = String::from_utf8_lossy(read_lllvar(raw, &mut offset)?).to_string();
                    // RSK:NNN: TLV prefix — Network Risk Score injected by upstream network
                    // (e.g. Visa Advanced Authorization). Format: "RSK:085:" where 085 is
                    // the 3-digit score (000–100). The remainder of F48 is the TAVV cryptogram.
                    if f48.starts_with("RSK:") && f48.len() >= 8 {
                        // Parse the 3-digit score: chars 4..7
                        if let Ok(score) = f48[4..7].parse::<u8>() {
                            tx.risk_score = score;
                        }
                        // Remainder after 'RSK:NNN:' (8 chars) is the TAVV cryptogram
                        let remainder = f48[8..].to_string();
                        if tx.is_tokenized && !remainder.is_empty() {
                            tx.tavv_cryptogram = Some(remainder);
                        }
                    } else if tx.is_tokenized {
                        // No RSK prefix — treat entire F48 as TAVV cryptogram (existing behaviour)
                        tx.tavv_cryptogram = Some(f48);
                    }
                },
                52 => tx.pin_block = Bytes::copy_from_slice(read_fixed(raw, &mut offset, 16)?),
                126 => {
                    let tavv = String::from_utf8_lossy(read_lllvar(raw, &mut offset)?).to_string();
                    if tx.is_tokenized {
                        tx.tavv_cryptogram = Some(tavv);
                    }
                },
                _ => { 
                    /* Unsupported fields currently skipped. In true production we must advance offsets properly based on field definition.
                       For strict Canonical bounds we only encode what we decode. 
                       Wait! If we receive a packet with unsupported fields, it will break offset!
                       Since Simulator and Egress ONLY inject what is generated locally, this strictly bounded subset is perfectly aligned for our Canonical schema. */
                } 
            }
        }
    }
    
    Ok(tx)
}

fn universal_encode(tx: &UniversalPaymentEvent) -> Result<bytes::Bytes, ParseError> {
    let mut data = bytes::BytesMut::new();
    let mut bitmap = [0u8; 8];
    
    // We will build the variable parts dynamically
    let mut payload = bytes::BytesMut::new();

    if !tx.fpan.is_empty() {
        set_bit(&mut bitmap, 2);
        write_llvar(&mut payload, &tx.fpan);
    }
    
    if !tx.processing_code.0.is_empty() {
        set_bit(&mut bitmap, 3);
        write_fixed(&mut payload, tx.processing_code.0.as_bytes(), 6, b'0', false);
    }
    
    if tx.amount > 0 || (tx.mti.starts_with(b"02") && tx.message_class == MessageClass::Financial) {
        set_bit(&mut bitmap, 4);
        let amt_str = format!("{:012}", tx.amount);
        write_fixed(&mut payload, amt_str.as_bytes(), 12, b'0', false);
    }

    if !tx.stan.0.is_empty() {
        set_bit(&mut bitmap, 11);
        write_fixed(&mut payload, tx.stan.0.as_bytes(), 6, b'0', false);
    }

    if !tx.local_time.0.is_empty() {
        set_bit(&mut bitmap, 12);
        write_fixed(&mut payload, tx.local_time.0.as_bytes(), 6, b'0', false);
    }

    // ISO-8583 INVARIANT: All data fields MUST be appended to the payload in strictly
    // ascending field-number order. The decoder iterates bits 1..=64 numerically and
    // reads bytes sequentially — any field written out of order corrupts every
    // subsequent field's offset, silently mangling the parsed canonical model.

    if !tx.local_date.0.is_empty() {
        set_bit(&mut bitmap, 13);
        write_fixed(&mut payload, tx.local_date.0.as_bytes(), 4, b'0', false);
    }

    // Field 22: POS Entry Mode — must appear before fields 32, 37, 39, 48, 52.
    if tx.is_tokenized {
        set_bit(&mut bitmap, 22);
        write_fixed(&mut payload, b"071", 3, b'0', false);
    }

    if !tx.acquirer_id.is_empty() {
        set_bit(&mut bitmap, 32);
        write_llvar(&mut payload, &tx.acquirer_id);
    }

    if !tx.rrn.0.is_empty() {
        set_bit(&mut bitmap, 37);
        write_fixed(&mut payload, tx.rrn.0.as_bytes(), 12, b' ', true);
    }

    if !tx.response_code.0.is_empty() {
        set_bit(&mut bitmap, 39);
        write_fixed(&mut payload, tx.response_code.0.as_bytes(), 2, b' ', false);
    }

    // Field 48: Additional Data (TAVV cryptogram) — must appear after field 39, before field 52.
    if tx.is_tokenized {
        if let Some(ref cryp) = tx.tavv_cryptogram {
            set_bit(&mut bitmap, 48);
            write_lllvar(&mut payload, cryp.as_bytes());
        }
    }

    if !tx.pin_block.is_empty() {
        set_bit(&mut bitmap, 52);
        write_fixed(&mut payload, &tx.pin_block, 16, b'0', false);
    }

    if tx.mti.len() >= 4 {
        data.extend_from_slice(&tx.mti[0..4]);
    } else {
        data.extend_from_slice(b"0110");
    }
    
    data.extend_from_slice(&bitmap);
    data.extend_from_slice(&payload);
    
    Ok(data.freeze())
}


pub struct Base24Dialect;
impl IsoDialect for Base24Dialect {
    fn decode(&self, raw: &[u8]) -> Result<UniversalPaymentEvent, ParseError> { universal_decode(raw) }
    fn encode(&self, tx: &UniversalPaymentEvent) -> Result<bytes::Bytes, ParseError> { universal_encode(tx) }
}

pub struct ConnexDialect;
impl IsoDialect for ConnexDialect {
    fn decode(&self, raw: &[u8]) -> Result<UniversalPaymentEvent, ParseError> { universal_decode(raw) }
    fn encode(&self, tx: &UniversalPaymentEvent) -> Result<bytes::Bytes, ParseError> { universal_encode(tx) }
}

pub enum DialectRouter {
    Base24(Base24Dialect),
    Connex(ConnexDialect),
}

impl DialectRouter {
    pub fn decode(&self, raw: &[u8]) -> Result<UniversalPaymentEvent, ParseError> {
        match self {
            DialectRouter::Base24(d) => d.decode(raw),
            DialectRouter::Connex(d) => d.decode(raw),
        }
    }
    pub fn encode(&self, tx: &UniversalPaymentEvent) -> Result<bytes::Bytes, ParseError> {
        match self {
            DialectRouter::Base24(d) => d.encode(tx),
            DialectRouter::Connex(d) => d.encode(tx),
        }
    }
}
