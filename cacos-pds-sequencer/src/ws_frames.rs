//! WebSocket frames for the subscribe-repos firehose stream.
//!
//! On-wire format: two DAG-CBOR objects back to back. The first is the
//! header (FrameType + optional message-type discriminator). The second is
//! the body.
//!
//! FrameType is encoded as `repr(i8)` so a node can branch on the first
//! byte of the frame (Message = 1, Error = -1).

use serde::{Deserialize, Serialize};
use serde_repr::{Deserialize_repr, Serialize_repr};

/// Frame op type tag. Corresponds to the `op` field in the header.
#[derive(Debug, Clone, PartialEq, Serialize_repr, Deserialize_repr)]
#[repr(i8)]
pub enum FrameType {
    Message = 1,
    Error = -1,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MessageFrameHeader {
    pub op: FrameType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub t: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ErrorFrameHeader {
    pub op: FrameType,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ErrorFrameBody {
    pub error: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// Body of a `#info` message frame, used to send `OutdatedCursor` and
/// similar non-error notices to the client.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InfoFrameBody {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum FrameHeader {
    Message(MessageFrameHeader),
    Error(ErrorFrameHeader),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CloseCode {
    Normal = 1000,
    Abnormal = 1006,
    Policy = 1008,
}

pub struct MessageFrameOpts {
    pub r#type: Option<String>,
}

pub trait Frame {
    fn get_op(&self) -> &FrameType;
    fn to_bytes(&self) -> anyhow::Result<Vec<u8>>;
    fn is_message(&self) -> bool;
    fn is_error(&self) -> bool;
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum FrameEnum {
    Error(ErrorFrame),
    Message(MessageFrame<serde_json::Value>),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MessageFrame<T> {
    pub header: MessageFrameHeader,
    pub body: T,
}

impl<T> MessageFrame<T> {
    pub fn new(body: T, opts: Option<MessageFrameOpts>) -> Self {
        let header = match opts {
            None => MessageFrameHeader {
                op: FrameType::Message,
                t: None,
            },
            Some(opts) => MessageFrameHeader {
                op: FrameType::Message,
                t: opts.r#type,
            },
        };
        Self { header, body }
    }

    pub fn get_type(&self) -> Option<&String> {
        self.header.t.as_ref()
    }
}

impl<T: serde::Serialize> Frame for MessageFrame<T> {
    fn get_op(&self) -> &FrameType {
        &self.header.op
    }

    fn to_bytes(&self) -> anyhow::Result<Vec<u8>> {
        let header = rsky_common::struct_to_cbor(&self.header)?;
        let body = serde_ipld_dagcbor::to_vec(&self.body)?;
        Ok([header, body].concat())
    }

    fn is_message(&self) -> bool {
        *self.get_op() == FrameType::Message
    }

    fn is_error(&self) -> bool {
        *self.get_op() == FrameType::Error
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ErrorFrame {
    pub header: ErrorFrameHeader,
    pub body: ErrorFrameBody,
}

impl ErrorFrame {
    pub fn new(body: ErrorFrameBody) -> Self {
        Self {
            header: ErrorFrameHeader {
                op: FrameType::Error,
            },
            body,
        }
    }

    pub fn get_code(&self) -> &String {
        &self.body.error
    }

    pub fn get_message(&self) -> Option<&String> {
        self.body.message.as_ref()
    }
}

impl Frame for ErrorFrame {
    fn get_op(&self) -> &FrameType {
        &self.header.op
    }

    fn to_bytes(&self) -> anyhow::Result<Vec<u8>> {
        let header = serde_ipld_dagcbor::to_vec(&self.header)?;
        let body = serde_ipld_dagcbor::to_vec(&self.body)?;
        Ok([header, body].concat())
    }

    fn is_message(&self) -> bool {
        *self.get_op() == FrameType::Message
    }

    fn is_error(&self) -> bool {
        *self.get_op() == FrameType::Error
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_cbor::Value as CborValue;

    fn decode_two(bytes: &[u8]) -> (CborValue, CborValue) {
        let mut values = serde_cbor::Deserializer::from_slice(bytes).into_iter::<CborValue>();
        let header = values.next().unwrap().unwrap();
        let body = values.next().unwrap().unwrap();
        assert!(values.next().is_none());
        (header, body)
    }

    fn get<'a>(map: &'a CborValue, key: &str) -> Option<&'a CborValue> {
        let CborValue::Map(map) = map else {
            panic!("expected cbor map");
        };
        map.get(&CborValue::Text(key.to_owned()))
    }

    #[test]
    fn info_frame_encodes_message_header_and_body() {
        let frame = MessageFrame::new(
            InfoFrameBody {
                name: "OutdatedCursor".to_owned(),
                message: Some("Requested cursor exceeded limit".to_owned()),
            },
            Some(MessageFrameOpts {
                r#type: Some("#info".to_owned()),
            }),
        );
        assert!(frame.is_message());
        assert!(!frame.is_error());
        assert_eq!(frame.get_type(), Some(&"#info".to_owned()));

        let (header, body) = decode_two(&frame.to_bytes().unwrap());
        assert_eq!(get(&header, "op"), Some(&CborValue::Integer(1)));
        assert_eq!(
            get(&header, "t"),
            Some(&CborValue::Text("#info".to_owned()))
        );
        assert_eq!(
            get(&body, "name"),
            Some(&CborValue::Text("OutdatedCursor".to_owned()))
        );
        assert!(get(&body, "message").is_some());
    }

    #[test]
    fn error_frame_encodes_negative_op() {
        let frame = ErrorFrame::new(ErrorFrameBody {
            error: "FutureCursor".to_owned(),
            message: Some("Cursor in the future.".to_owned()),
        });
        assert!(frame.is_error());
        assert!(!frame.is_message());
        assert_eq!(frame.get_code(), "FutureCursor");
        assert_eq!(
            frame.get_message(),
            Some(&"Cursor in the future.".to_owned())
        );

        let (header, body) = decode_two(&frame.to_bytes().unwrap());
        assert_eq!(get(&header, "op"), Some(&CborValue::Integer(-1)));
        assert!(get(&header, "t").is_none());
        assert_eq!(
            get(&body, "error"),
            Some(&CborValue::Text("FutureCursor".to_owned()))
        );
    }
}
