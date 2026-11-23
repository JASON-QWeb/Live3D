use super::*;
use flate2::{Compression, write::GzEncoder};
use std::io::Write;

#[test]
fn parses_cookie_pairs_like_the_original() {
    assert_eq!(
        parse_cookie(" a=1 ; b = x=y ;bad; ; msToken= t "),
        vec![
            ("a".into(), "1".into()),
            ("b".into(), "x=y".into()),
            ("msToken".into(), "t".into())
        ]
    );
}

#[test]
fn ms_token_uses_the_original_charset() {
    let token = ms_token();
    assert_eq!(token.len(), 172);
    assert!(
        token
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    );
}

#[test]
fn decodes_gzip_frames_acks_and_live_end() {
    let target = Target {
        room: "1".into(),
        port: 0,
        program: String::new(),
        cookie: None,
    };
    let mut session = Session::new(&target).unwrap();
    let chat = ChatMessage {
        common: Some(super::super::proto::Common { msg_id: 77 }),
        user: Some(super::super::proto::User {
            id: 5,
            nickname: "小明".into(),
            ..Default::default()
        }),
        content: "主播好".into(),
    };
    let response = Response {
        messages: vec![
            super::super::proto::Message {
                method: "WebcastChatMessage".into(),
                payload: chat.encode_to_vec(),
                msg_id: 77,
            },
            super::super::proto::Message {
                method: "WebcastControlMessage".into(),
                payload: ControlMessage { action: LIVE_END }.encode_to_vec(),
                msg_id: 78,
            },
        ],
        cursor: "c1".into(),
        internal_ext: "from-response".into(),
        heartbeat_duration: 15,
        need_ack: true,
        push_server_v2: "webcast5-ws-web-lf.douyin.com".into(),
        ..Default::default()
    };
    let mut gz = GzEncoder::new(Vec::new(), Compression::default());
    gz.write_all(&response.encode_to_vec()).unwrap();
    let frame = PushFrame {
        log_id: 9,
        headers: vec![
            super::super::proto::PushHeader {
                key: "compress_type".into(),
                value: "gzip".into(),
            },
            super::super::proto::PushHeader {
                key: "im-internal_ext".into(),
                value: "from-header".into(),
            },
        ],
        payload_type: "msg".into(),
        payload: gz.finish().unwrap(),
    };
    let decoded = decode(&mut session, &frame).unwrap();
    assert!(decoded.ended);
    assert_eq!(decoded.ack, Some(PushFrame::ack(9, "from-header")));
    let [Event::Comment(c)] = decoded.events.as_slice() else {
        panic!("one chat event")
    };
    assert_eq!(
        (c.id.as_str(), c.author.as_str(), c.text.as_str()),
        ("77", "小明", "主播好")
    );
    assert_eq!(c.viewer_id.as_deref(), Some("5"));
    assert_eq!(session.cursor, "c1");
    assert_eq!(session.heartbeat, Duration::from_secs(15));
    assert_eq!(
        session.push_url,
        "wss://webcast5-ws-web-lf.douyin.com/webcast/im/push/v2/"
    );
}

#[test]
fn websocket_url_parses_as_a_request() {
    let target = Target {
        room: "1".into(),
        port: 0,
        program: String::new(),
        cookie: None,
    };
    let mut session = Session::new(&target).unwrap();
    session.info.room_id = "7659792511015177001".into();
    session.info.push_id = "7659797852999091746".into();
    let url = session.websocket_url().unwrap();
    assert!(url.starts_with(params::DEFAULT_PUSH_URL));
    url.as_str().into_client_request().unwrap();
}
