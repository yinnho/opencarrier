//! Minimal protobuf codec for the Feishu WS v2 protocol (pbbp2.Frame).
//!
//! Wire format:
//! ```protobuf
//! message Frame {
//!   uint64  SeqID           = 1;
//!   uint64  LogID           = 2;
//!   int32   service         = 3;
//!   int32   method          = 4;   // 0=control, 1=data
//!   repeated Header headers = 5;
//!   string  payloadEncoding = 6;
//!   string  payloadType     = 7;
//!   bytes   payload         = 8;
//!   string  LogIDNew        = 9;
//! }
//! message Header {
//!   string key   = 1;
//!   string value = 2;
//! }
//! ```

use std::collections::HashMap;
use std::time::Instant;

/// Control frame (ping/pong/handshake).
pub const METHOD_CONTROL: i32 = 0;
/// Data frame (event delivery).
pub const METHOD_DATA: i32 = 1;

// ---------------------------------------------------------------------------
// Varint helpers
// ---------------------------------------------------------------------------

fn encode_varint(value: u64) -> Vec<u8> {
    let mut buf = Vec::new();
    let mut v = value;
    loop {
        let byte = (v & 0x7F) as u8;
        v >>= 7;
        if v == 0 {
            buf.push(byte);
            break;
        }
        buf.push(byte | 0x80);
    }
    buf
}

fn decode_varint(data: &[u8], offset: &mut usize) -> Option<u64> {
    let mut result: u64 = 0;
    let mut shift: u32 = 0;
    loop {
        let byte = data.get(*offset)?;
        *offset += 1;
        result |= ((byte & 0x7F) as u64) << shift;
        if byte & 0x80 == 0 {
            return Some(result);
        }
        shift += 7;
        if shift >= 64 {
            return None;
        }
    }
}

// ---------------------------------------------------------------------------
// Length-delimited helpers
// ---------------------------------------------------------------------------

fn encode_bytes(field_number: u32, data: &[u8]) -> Vec<u8> {
    let tag = ((field_number << 3) | 2) as u64;
    let mut buf = encode_varint(tag);
    buf.extend(encode_varint(data.len() as u64));
    buf.extend_from_slice(data);
    buf
}

fn encode_varint_field(field_number: u32, value: u64) -> Vec<u8> {
    let tag = ((field_number << 3) | 0) as u64;
    let mut buf = encode_varint(tag);
    buf.extend(encode_varint(value));
    buf
}

fn read_length_delimited(data: &[u8], offset: &mut usize) -> Option<Vec<u8>> {
    let len = decode_varint(data, offset)? as usize;
    if *offset + len > data.len() {
        return None;
    }
    let bytes = data[*offset..*offset + len].to_vec();
    *offset += len;
    Some(bytes)
}

// ---------------------------------------------------------------------------
// Header
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct Pbbp2Header {
    pub key: String,
    pub value: String,
}

impl Pbbp2Header {
    pub fn new(key: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            value: value.into(),
        }
    }

    fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        if !self.key.is_empty() {
            buf.extend(encode_bytes(1, self.key.as_bytes()));
        }
        if !self.value.is_empty() {
            buf.extend(encode_bytes(2, self.value.as_bytes()));
        }
        buf
    }

    fn decode(data: &[u8]) -> Option<Self> {
        let mut header = Self::default();
        let mut offset = 0;
        while offset < data.len() {
            let tag = decode_varint(data, &mut offset)?;
            let field_number = (tag >> 3) as u32;
            match field_number {
                1 => {
                    let bytes = read_length_delimited(data, &mut offset)?;
                    header.key = String::from_utf8_lossy(&bytes).into_owned();
                }
                2 => {
                    let bytes = read_length_delimited(data, &mut offset)?;
                    header.value = String::from_utf8_lossy(&bytes).into_owned();
                }
                _ => {
                    skip_field(tag, data, &mut offset)?;
                }
            }
        }
        Some(header)
    }
}

// ---------------------------------------------------------------------------
// Frame
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct Pbbp2Frame {
    pub seq_id: u64,
    pub log_id: u64,
    pub service: i32,
    pub method: i32,
    pub headers: Vec<Pbbp2Header>,
    pub payload_encoding: String,
    pub payload_type: String,
    pub payload: Vec<u8>,
    pub log_id_new: String,
}

impl Pbbp2Frame {
    /// Encode this frame to protobuf wire format.
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        if self.seq_id != 0 {
            buf.extend(encode_varint_field(1, self.seq_id));
        }
        if self.log_id != 0 {
            buf.extend(encode_varint_field(2, self.log_id));
        }
        if self.service != 0 {
            buf.extend(encode_varint_field(3, self.service as u64));
        }
        if self.method != 0 {
            buf.extend(encode_varint_field(4, self.method as u64));
        }
        for header in &self.headers {
            let encoded = header.encode();
            buf.extend(encode_bytes(5, &encoded));
        }
        if !self.payload_encoding.is_empty() {
            buf.extend(encode_bytes(6, self.payload_encoding.as_bytes()));
        }
        if !self.payload_type.is_empty() {
            buf.extend(encode_bytes(7, self.payload_type.as_bytes()));
        }
        if !self.payload.is_empty() {
            buf.extend(encode_bytes(8, &self.payload));
        }
        if !self.log_id_new.is_empty() {
            buf.extend(encode_bytes(9, self.log_id_new.as_bytes()));
        }
        buf
    }

    /// Decode a frame from protobuf wire format.
    pub fn decode(data: &[u8]) -> Option<Self> {
        let mut frame = Self::default();
        let mut offset = 0;
        while offset < data.len() {
            let tag = decode_varint(data, &mut offset)?;
            let field_number = (tag >> 3) as u32;
            match field_number {
                1 => frame.seq_id = decode_varint(data, &mut offset)?,
                2 => frame.log_id = decode_varint(data, &mut offset)?,
                3 => frame.service = decode_varint(data, &mut offset)? as i32,
                4 => frame.method = decode_varint(data, &mut offset)? as i32,
                5 => {
                    let bytes = read_length_delimited(data, &mut offset)?;
                    if let Some(h) = Pbbp2Header::decode(&bytes) {
                        frame.headers.push(h);
                    }
                }
                6 => {
                    let bytes = read_length_delimited(data, &mut offset)?;
                    frame.payload_encoding = String::from_utf8_lossy(&bytes).into_owned();
                }
                7 => {
                    let bytes = read_length_delimited(data, &mut offset)?;
                    frame.payload_type = String::from_utf8_lossy(&bytes).into_owned();
                }
                8 => {
                    let bytes = read_length_delimited(data, &mut offset)?;
                    frame.payload = bytes;
                }
                9 => {
                    let bytes = read_length_delimited(data, &mut offset)?;
                    frame.log_id_new = String::from_utf8_lossy(&bytes).into_owned();
                }
                _ => {
                    skip_field(tag, data, &mut offset)?;
                }
            }
        }
        Some(frame)
    }

    /// Find a header value by key.
    pub fn header(&self, key: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|h| h.key == key)
            .map(|h| h.value.as_str())
    }

    /// Build a ping frame (application-level heartbeat).
    pub fn ping(service: i32) -> Self {
        Self {
            service,
            method: METHOD_CONTROL,
            headers: vec![Pbbp2Header::new("type", "ping")],
            ..Default::default()
        }
    }

    /// Build an ACK frame for an event data frame.
    pub fn ack_for(event: &Pbbp2Frame, biz_rt_ms: i64) -> Self {
        let mut headers = event.headers.clone();
        headers.push(Pbbp2Header::new("biz_rt", biz_rt_ms.to_string()));
        let ack_payload = serde_json::json!({"code": 200}).to_string();
        Self {
            seq_id: event.seq_id,
            log_id: event.log_id,
            service: event.service,
            method: event.method,
            headers,
            payload: ack_payload.into_bytes(),
            ..Default::default()
        }
    }
}

// ---------------------------------------------------------------------------
// Skip unknown protobuf field
// ---------------------------------------------------------------------------

fn skip_field(tag: u64, data: &[u8], offset: &mut usize) -> Option<()> {
    let wire_type = tag & 0x07;
    match wire_type {
        0 => {
            decode_varint(data, offset)?;
        }
        1 => {
            *offset += 8;
        }
        2 => {
            read_length_delimited(data, offset)?;
        }
        5 => {
            *offset += 4;
        }
        _ => return None,
    }
    Some(())
}

// ---------------------------------------------------------------------------
// Fragment reassembly cache
// ---------------------------------------------------------------------------

struct FragmentParts {
    total: usize,
    parts: Vec<Option<Vec<u8>>>,
    first_seen: Instant,
}

/// Reassembles fragmented Feishu WS event frames.
///
/// The server may split large events into multiple frames identified by
/// `message_id` header. Each fragment has `seq` (index) and `sum` (total)
/// headers. All fragments must arrive before the event is dispatched.
pub struct FragmentCache {
    fragments: HashMap<String, FragmentParts>,
}

impl FragmentCache {
    pub fn new() -> Self {
        Self {
            fragments: HashMap::new(),
        }
    }

    /// Try to add a fragment. Returns `Some(complete_payload)` when all parts collected.
    /// Returns `None` if more fragments are needed.
    pub fn add(
        &mut self,
        message_id: &str,
        seq: usize,
        sum: usize,
        payload: Vec<u8>,
    ) -> Option<Vec<u8>> {
        // Single-frame message
        if sum <= 1 {
            return Some(payload);
        }

        // Evict expired entries
        self.fragments.retain(|_, v| v.first_seen.elapsed().as_secs() < 10);

        let entry = self
            .fragments
            .entry(message_id.to_string())
            .or_insert_with(|| FragmentParts {
                total: sum,
                parts: vec![None; sum],
                first_seen: Instant::now(),
            });

        if entry.total != sum {
            *entry = FragmentParts {
                total: sum,
                parts: vec![None; sum],
                first_seen: Instant::now(),
            };
        }
        if seq < entry.parts.len() {
            entry.parts[seq] = Some(payload);
        }

        if entry.parts.iter().all(|p| p.is_some()) {
            let parts = self.fragments.remove(message_id).unwrap();
            let mut complete = Vec::new();
            for part in parts.parts.into_iter().flatten() {
                complete.extend(part);
            }
            return Some(complete);
        }

        None
    }
}
