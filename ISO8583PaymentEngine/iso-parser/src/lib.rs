use nom::{
    bytes::complete::take,
    combinator::map_res,
    sequence::tuple,
    IResult,
};
use std::str;

#[derive(Debug, PartialEq)]
pub struct Iso8583Message {
    pub mti: String,
    pub primary_bitmap: Vec<u8>,
}

pub fn parse_mti(input: &[u8]) -> IResult<&[u8], String> {
    map_res(
        take(4usize),
        |s: &[u8]| str::from_utf8(s).map(|s| s.to_string())
    )(input)
}

pub fn parse_primary_bitmap(input: &[u8]) -> IResult<&[u8], Vec<u8>> {
    let (input, bitmap) = take(8usize)(input)?;
    Ok((input, bitmap.to_vec()))
}

pub fn parse_iso8583(input: &[u8]) -> IResult<&[u8], Iso8583Message> {
    let (input, (mti, bitmap)) = tuple((parse_mti, parse_primary_bitmap))(input)?;
    
    if mti != "0100" && mti != "0110" {
        return Err(nom::Err::Error(nom::error::Error::new(input, nom::error::ErrorKind::Tag)));
    }
    
    Ok((input, Iso8583Message { mti, primary_bitmap: bitmap }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_0100_authorization_request() {
        let mti = b"0100";
        let bitmap = [0xFF, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
        let mut input = Vec::new();
        input.extend_from_slice(mti);
        input.extend_from_slice(&bitmap);

        let result = parse_iso8583(&input);
        assert!(result.is_ok(), "Failed to parse ISO message");

        let (remaining, msg) = result.unwrap();
        assert!(remaining.is_empty(), "Expected empty remainder after parsing MTI and bitmap");
        assert_eq!(msg.mti, "0100");
        assert_eq!(msg.primary_bitmap, bitmap.to_vec());
    }
}
