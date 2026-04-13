use bytes::{Buf, BufMut, BytesMut};
use tokio_util::codec::{Decoder, Encoder};
use std::io;

pub struct Iso8583Codec;

impl Decoder for Iso8583Codec {
    type Item = BytesMut;
    type Error = io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        if src.len() < 2 {
            return Ok(None);
        }

        let mut length_bytes = [0u8; 2];
        length_bytes.copy_from_slice(&src[..2]);
        let length = u16::from_be_bytes(length_bytes) as usize;

        if src.len() < 2 + length {
            src.reserve(2 + length - src.len());
            return Ok(None);
        }

        src.advance(2);
        let payload = src.split_to(length);
        Ok(Some(payload))
    }
}

impl Encoder<BytesMut> for Iso8583Codec {
    type Error = io::Error;

    fn encode(&mut self, item: BytesMut, dst: &mut BytesMut) -> Result<(), Self::Error> {
        let len = item.len() as u16;
        dst.reserve(2 + item.len());
        dst.put_u16(len);
        dst.extend_from_slice(&item);
        Ok(())
    }
}
