use payment_proto::Transaction;

pub trait IsoAdapter {
    fn to_iso8583(&self) -> Vec<u8>;
}

impl IsoAdapter for Transaction {
    fn to_iso8583(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(self.mti.as_bytes());
        bytes.extend_from_slice(&[0xFF; 8]); // Mock Bitmap Fixed
        bytes
    }
}
