use bytes::{BufMut, BytesMut};
use std::io;
use tokio_util::codec::{Decoder, Encoder};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThalesMessage {
    pub header: String,
    pub command: String,
    pub payload: String,
}

pub struct ThalesCodec;

impl Decoder for ThalesCodec {
    type Item = ThalesMessage;
    type Error = io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        if src.len() < 2 {
            return Ok(None);
        }

        let mut length_bytes = [0u8; 2];
        length_bytes.copy_from_slice(&src[0..2]);
        let length = u16::from_be_bytes(length_bytes) as usize;

        if src.len() < 2 + length {
            src.reserve(2 + length - src.len());
            return Ok(None);
        }

        let _ = src.split_to(2); // Remove length bytes
        let data = src.split_to(length);
        
        // Thales payloads are typically EBCDIC or ASCII. We'll assume ASCII/UTF-8 here.
        let msg_str = String::from_utf8(data.to_vec())
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("Invalid UTF-8: {}", e)))?;

        // Format is typically Header (4) + Command (2) + Payload
        if msg_str.len() < 6 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "Thales message too short"));
        }

        let header = msg_str[0..4].to_string();
        let command = msg_str[4..6].to_string();
        let payload = msg_str[6..].to_string();

        Ok(Some(ThalesMessage {
            header,
            command,
            payload,
        }))
    }
}

impl Encoder<ThalesMessage> for ThalesCodec {
    type Error = io::Error;

    fn encode(&mut self, item: ThalesMessage, dst: &mut BytesMut) -> Result<(), Self::Error> {
        let payload_len = item.header.len() + item.command.len() + item.payload.len();
        if payload_len > u16::MAX as usize {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "Message too long"));
        }

        dst.reserve(2 + payload_len);
        dst.put_u16(payload_len as u16);
        dst.put_slice(item.header.as_bytes());
        dst.put_slice(item.command.as_bytes());
        dst.put_slice(item.payload.as_bytes());
        Ok(())
    }
}
