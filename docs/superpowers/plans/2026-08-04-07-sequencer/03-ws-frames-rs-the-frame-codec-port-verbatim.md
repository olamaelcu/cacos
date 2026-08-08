# Task 3: `ws_frames.rs` — the frame codec (port verbatim)

**Files:**
- Create: `pds/src/sequencer/ws_frames.rs`
- Test: `pds/src/sequencer/ws_frames.rs` (`#[cfg(test)] mod tests`)
- Modify: `pds/Cargo.toml` (deps below)

Port sources: the rsky-pds websocket frame codec is now reached through the git-pinned `rsky-common` / `rsky-lexicon` crates (`Cargo.toml:8-15`). The CBOR codec shape (`serde_ipld_dagcbor::to_vec` + `serde_bytes`) is identical to the reference. Frames are `CBOR(header) ++ CBOR(body)`; header `{op: 1, t: "#commit"|"#identity"|"#account"|"#sync"|"#info"}` or `{op: -1}` for errors.

- [ ] **Step 1: Add dependencies to `pds/Cargo.toml`**

```toml
# [dependencies] — add ONLY what is NOT already in pds/Cargo.toml or Cargo.toml:7-61.
serde_repr = "0.1"        # FrameType repr(i8) — not yet in pds/Cargo.toml
# serde_ipld_dagcbor is already a dev-dep on pds/Cargo.toml:74; PROMOTE it to [dependencies] (Task 3 needs it for runtime `frame.to_bytes()`)
# [dev-dependencies] — add
serde_cbor = "0.11"       # tests assert CBOR wire shape — not yet in pds/Cargo.toml
```

- [ ] **Step 2: Write the failing tests** (ported from the git-pinned rsky fork's websocket frame codec — same shape as the reference)

Create `pds/src/sequencer/ws_frames.rs` containing ONLY this test module for now:

```rust
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
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test -p cacos-pds sequencer::ws_frames::tests`
Expected: FAIL — `cannot find type 'MessageFrame'` (and the other frame types) in `sequencer::ws_frames`.

- [ ] **Step 4: Implement `pds/src/sequencer/ws_frames.rs`** (full port; keep the test module from Step 2 appended)

```rust
use anyhow::Result;
use rsky_common::struct_to_cbor;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use serde_repr::{Deserialize_repr, Serialize_repr};

#[derive(Debug, Clone, PartialEq, Serialize_repr, Deserialize_repr)]
#[repr(i8)]
pub enum FrameType {
    Message = 1,
    Error = -1,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MessageFrameHeader {
    pub op: FrameType,     // Frame op
    pub t: Option<String>, // Message body type discriminator
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ErrorFrameHeader {
    pub op: FrameType, // Frame op
    // `t` Should not be included in header if op is -1.
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ErrorFrameBody {
    pub error: String,           // Error code
    pub message: Option<String>, // Error message
}

/// Body of a `#info` message frame on a subscription stream.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InfoFrameBody {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum FrameHeader {
    MessageFrameHeader(MessageFrameHeader),
    ErrorFrameHeader(ErrorFrameHeader),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CloseCode {
    Normal = 1000,
    Abnormal = 1006,
    Policy = 1008,
}

pub trait Frame {
    fn get_op(&self) -> &FrameType;

    fn to_bytes(&self) -> Result<Vec<u8>>;

    fn is_message(&self) -> bool;

    fn is_error(&self) -> bool;
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum FrameEnum {
    ErrorFrame(ErrorFrame), // Intentionally try to decode as Error first
    MessageFrame(MessageFrame<Value>),
}

pub struct MessageFrameOpts {
    pub r#type: Option<String>,
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

    fn to_bytes(&self) -> Result<Vec<u8>> {
        Ok([struct_to_cbor(&self.header)?, struct_to_cbor(&self.body)?].concat())
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

    fn to_bytes(&self) -> Result<Vec<u8>> {
        Ok([
            serde_ipld_dagcbor::to_vec(&self.header)?,
            serde_ipld_dagcbor::to_vec(&self.body)?,
        ]
        .concat())
    }

    fn is_message(&self) -> bool {
        *self.get_op() == FrameType::Message
    }

    fn is_error(&self) -> bool {
        *self.get_op() == FrameType::Error
    }
}
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p cacos-pds sequencer::ws_frames::tests`
Expected: `test result: ok. 2 passed`

- [ ] **Step 6: Commit**

```bash
git add pds/Cargo.toml pds/src/sequencer/ws_frames.rs
git commit -m "feat(sequencer): port websocket frame codec (ws_frames.rs)"
```
