//! The Douyin webcast protobuf messages Vorela reads. Field numbers come
//! from douyinlive-proto v0.1.3 (`new_douyin.proto`); fields not listed are
//! skipped when decoding.

use prost::Message as _;

#[derive(Clone, PartialEq, prost::Message)]
pub struct PushHeader {
    #[prost(string, tag = "1")]
    pub key: String,
    #[prost(string, tag = "2")]
    pub value: String,
}

/// `Webcast.Im.PushFrame`: every WebSocket frame, both ways.
#[derive(Clone, PartialEq, prost::Message)]
pub struct PushFrame {
    #[prost(uint64, tag = "2")]
    pub log_id: u64,
    #[prost(message, repeated, tag = "5")]
    pub headers: Vec<PushHeader>,
    #[prost(string, tag = "7")]
    pub payload_type: String,
    #[prost(bytes = "vec", tag = "8")]
    pub payload: Vec<u8>,
}
impl PushFrame {
    pub fn header(&self, key: &str) -> &str {
        self.headers
            .iter()
            .find(|h| h.key == key)
            .map_or("", |h| h.value.as_str())
    }
    /// The heartbeat the web client sends.
    pub fn heartbeat() -> Vec<u8> {
        Self {
            payload_type: "hb".into(),
            ..Default::default()
        }
        .encode_to_vec()
    }
    /// The acknowledgement for a frame, echoing its internal_ext.
    pub fn ack(log_id: u64, internal_ext: &str) -> Vec<u8> {
        Self {
            log_id,
            payload_type: "ack".into(),
            payload: internal_ext.as_bytes().to_vec(),
            ..Default::default()
        }
        .encode_to_vec()
    }
}

/// `Webcast.Im.Response`: from im/fetch, and inside each "msg" frame.
#[derive(Clone, PartialEq, prost::Message)]
pub struct Response {
    #[prost(message, repeated, tag = "1")]
    pub messages: Vec<Message>,
    #[prost(string, tag = "2")]
    pub cursor: String,
    #[prost(string, tag = "5")]
    pub internal_ext: String,
    #[prost(uint64, tag = "8")]
    pub heartbeat_duration: u64,
    #[prost(bool, tag = "9")]
    pub need_ack: bool,
    #[prost(string, tag = "10")]
    pub push_server: String,
    #[prost(string, tag = "13")]
    pub proxy_server: String,
    #[prost(string, tag = "14")]
    pub push_server_v2: String,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct Message {
    #[prost(string, tag = "1")]
    pub method: String,
    #[prost(bytes = "vec", tag = "2")]
    pub payload: Vec<u8>,
    #[prost(uint64, tag = "3")]
    pub msg_id: u64,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct Common {
    #[prost(uint64, tag = "2")]
    pub msg_id: u64,
}

/// `Webcast.Data.User`.
#[derive(Clone, PartialEq, prost::Message)]
pub struct User {
    #[prost(uint64, tag = "1")]
    pub id: u64,
    #[prost(string, tag = "3")]
    pub nickname: String,
    #[prost(string, tag = "46")]
    pub sec_uid: String,
    #[prost(string, tag = "1028")]
    pub id_str: String,
}
impl User {
    /// A stable viewer id: id_str, id or sec_uid.
    pub fn viewer(&self) -> Option<String> {
        [
            self.id_str.trim().to_owned(),
            if self.id != 0 {
                self.id.to_string()
            } else {
                String::new()
            },
            self.sec_uid.trim().to_owned(),
        ]
        .into_iter()
        .find(|s| !s.is_empty() && s != "0")
    }
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct ChatMessage {
    #[prost(message, optional, tag = "1")]
    pub common: Option<Common>,
    #[prost(message, optional, tag = "2")]
    pub user: Option<User>,
    #[prost(string, tag = "3")]
    pub content: String,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct MemberMessage {
    #[prost(message, optional, tag = "1")]
    pub common: Option<Common>,
    #[prost(message, optional, tag = "2")]
    pub user: Option<User>,
    #[prost(uint64, tag = "10")]
    pub action: u64,
}

/// `Webcast.Data.GiftStruct`.
#[derive(Clone, PartialEq, prost::Message)]
pub struct GiftStruct {
    #[prost(string, tag = "16")]
    pub name: String,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct GiftMessage {
    #[prost(message, optional, tag = "1")]
    pub common: Option<Common>,
    #[prost(uint64, tag = "4")]
    pub group_count: u64,
    #[prost(uint64, tag = "5")]
    pub repeat_count: u64,
    #[prost(uint64, tag = "6")]
    pub combo_count: u64,
    #[prost(message, optional, tag = "7")]
    pub user: Option<User>,
    #[prost(int32, tag = "9")]
    pub repeat_end: i32,
    #[prost(uint64, tag = "11")]
    pub group_id: u64,
    #[prost(message, optional, tag = "15")]
    pub gift: Option<GiftStruct>,
    #[prost(uint64, tag = "29")]
    pub total_count: u64,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct ControlMessage {
    #[prost(uint64, tag = "2")]
    pub action: u64,
}

#[cfg(test)]
mod tests {
    use super::PushFrame;

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn frames_match_the_go_encoding() {
        // Bytes from google.golang.org/protobuf marshalling the same frames.
        assert_eq!(hex(&PushFrame::heartbeat()), "3a026862");
        assert_eq!(
            hex(&PushFrame::ack(
                7312345678901234567,
                "internal_src:dim|seq:1"
            )),
            "10879795cef0f4aabd653a0361636b4216696e7465726e616c5f7372633a64696d7c7365713a31"
        );
    }
}
