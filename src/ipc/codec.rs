use bytes::{Buf, BufMut, BytesMut};
use tokio_util::codec::{Decoder, Encoder};

use super::MAX_MESSAGE_SIZE;

/// Length-delimited codec for IPC message framing.
/// Wire format: [4-byte BE length][payload]
pub struct MessageCodec;

impl Decoder for MessageCodec {
    type Item = BytesMut;
    type Error = std::io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        if src.len() < 4 {
            return Ok(None);
        }

        let length = u32::from_be_bytes([src[0], src[1], src[2], src[3]]) as usize;

        if length > MAX_MESSAGE_SIZE {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("Message too large: {} bytes (max {})", length, MAX_MESSAGE_SIZE),
            ));
        }

        if src.len() < 4 + length {
            src.reserve(4 + length - src.len());
            return Ok(None);
        }

        src.advance(4);
        Ok(Some(src.split_to(length)))
    }
}

impl Encoder<bytes::Bytes> for MessageCodec {
    type Error = std::io::Error;

    fn encode(&mut self, item: bytes::Bytes, dst: &mut BytesMut) -> Result<(), Self::Error> {
        if item.len() > MAX_MESSAGE_SIZE {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("Message too large: {} bytes (max {})", item.len(), MAX_MESSAGE_SIZE),
            ));
        }

        dst.reserve(4 + item.len());
        dst.put_u32(item.len() as u32);
        dst.extend_from_slice(&item);
        Ok(())
    }
}

/// Tagged frame codec for stream mode.
/// Wire format: [4-byte BE length][1-byte type tag][payload]
pub struct StreamCodec;

#[derive(Debug)]
pub struct StreamFrame {
    pub frame_type: u8,
    pub payload: bytes::Bytes,
}

impl StreamFrame {
    pub fn pty_data(data: bytes::Bytes) -> Self {
        Self {
            frame_type: super::FRAME_PTY_DATA,
            payload: data,
        }
    }

    pub fn control(json: bytes::Bytes) -> Self {
        Self {
            frame_type: super::FRAME_CONTROL,
            payload: json,
        }
    }
}

impl Decoder for StreamCodec {
    type Item = StreamFrame;
    type Error = std::io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        if src.len() < 4 {
            return Ok(None);
        }

        let length = u32::from_be_bytes([src[0], src[1], src[2], src[3]]) as usize;

        if length > MAX_MESSAGE_SIZE {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Message too large",
            ));
        }

        if length < 1 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Stream frame must have at least 1 byte (type tag)",
            ));
        }

        if src.len() < 4 + length {
            src.reserve(4 + length - src.len());
            return Ok(None);
        }

        src.advance(4);
        let frame_type = src[0];
        src.advance(1);
        let payload = src.split_to(length - 1).freeze();

        Ok(Some(StreamFrame {
            frame_type,
            payload,
        }))
    }
}

impl Encoder<StreamFrame> for StreamCodec {
    type Error = std::io::Error;

    fn encode(&mut self, item: StreamFrame, dst: &mut BytesMut) -> Result<(), Self::Error> {
        let total_len = 1 + item.payload.len();
        if total_len > MAX_MESSAGE_SIZE {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Message too large",
            ));
        }

        dst.reserve(4 + total_len);
        dst.put_u32(total_len as u32);
        dst.put_u8(item.frame_type);
        dst.extend_from_slice(&item.payload);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_codec_roundtrip() {
        let mut codec = MessageCodec;
        let mut buf = BytesMut::new();

        let msg = bytes::Bytes::from(r#"{"cmd":"ls"}"#);
        codec.encode(msg.clone(), &mut buf).unwrap();

        let decoded = codec.decode(&mut buf).unwrap().unwrap();
        assert_eq!(&decoded[..], &msg[..]);
    }

    #[test]
    fn test_stream_codec_roundtrip() {
        let mut codec = StreamCodec;
        let mut buf = BytesMut::new();

        let frame = StreamFrame::pty_data(bytes::Bytes::from("hello"));
        codec.encode(frame, &mut buf).unwrap();

        let decoded = codec.decode(&mut buf).unwrap().unwrap();
        assert_eq!(decoded.frame_type, super::super::FRAME_PTY_DATA);
        assert_eq!(&decoded.payload[..], b"hello");
    }
}
