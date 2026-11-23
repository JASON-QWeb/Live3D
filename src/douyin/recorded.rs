//! Replays what douyinLive's Go code returned on 4,006 inputs against the Rust
//! port. The cases were recorded with rust-anything from douyinLive commit
//! af34f7c; `third_party/douyinLive-recording/README.md` says how to record
//! them again after the original changes.

use super::{page, params, params::Screen, sign};
use serde_json::{Value, json};

type Run = fn(&[Value]) -> Result<Value, String>;

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or_default()
}
fn n(v: &Value) -> i64 {
    v.as_i64().unwrap_or_default()
}
fn b(v: &Value) -> Result<u8, String> {
    u8::try_from(n(v)).map_err(|e| e.to_string())
}

fn snapshot(info: &page::RoomInfo) -> String {
    [
        info.room_id.as_str(),
        &info.push_id,
        &info.live_name,
        &info.title,
        &info.avatar,
        &info.anchor_only.to_string(),
    ]
    .join("|")
}

/// The recorded function's result, shaped like the Go wrapper's.
fn call(name: &str, a: &[Value]) -> Result<Value, String> {
    let screen = |i: usize| Screen {
        width: n(&a[i]),
        height: n(&a[i + 1]),
    };
    let text = |t: String| Ok(json!(t));
    let run: Run = match name {
        "SignWithValues" => |a| {
            sign::websocket_signature_with(s(&a[0]), b(&a[1])?, a[2] == true, b(&a[3])?, b(&a[4])?)
                .map(Value::from)
        },
        "AbSignAt" => |a| Ok(json!(sign::a_bogus_at(s(&a[0]), s(&a[1]), n(&a[2])))),
        "SM3Hex" => |a| Ok(json!(sign::sm3_hex(s(&a[0]).as_bytes()))),
        "GoldenWSStub" => |a| Ok(json!(params::websocket_stub(s(&a[0]), s(&a[1])))),
        "GoldenInternalExt" => |a| {
            Ok(json!(params::default_internal_ext(
                s(&a[0]),
                s(&a[1]),
                n(&a[2])
            )))
        },
        "GoldenPushURL" => |a| Ok(json!(params::push_url(s(&a[0])))),
        "GoldenPingInterval" => |a| Ok(json!(params::ping_interval(s(&a[0])))),
        "GoldenChromeVersion" => |a| Ok(json!(params::chrome_version(s(&a[0])))),
        "GoldenQueryEscape" => |a| {
            let v = s(&a[0]);
            Ok(json!(format!(
                "{}\t{}",
                params::query_escape_value(v),
                params::query_escape(v)
            )))
        },
        "GoldenValidateLiveID" => |a| {
            params::live_id(s(&a[0]))
                .map(Value::from)
                .ok_or_else(|| "直播间标识无效".to_owned())
        },
        "GoldenRoomEnter" => |a| {
            let body = s(&a[0]);
            let info = page::room_enter(body).map_err(str::to_owned)?;
            let live = page::room_enter_live(body);
            Ok(json!(format!(
                "{}|live={}|known={}",
                snapshot(&info),
                live == Some(true),
                live.is_some()
            )))
        },
        "GoldenLivePage" => |a| {
            let st = page::live_page(s(&a[0]));
            Ok(json!(format!(
                "{}|uid={}|anchor={}|live={}|known={}|valid={}",
                snapshot(&st.info),
                st.user_unique_id,
                st.has_anchor,
                st.live,
                st.status_known,
                st.known()
            )))
        },
        "GoldenWSQuery" => {
            return text(params::websocket_query(
                s(&a[0]),
                s(&a[1]),
                s(&a[2]),
                s(&a[3]),
                s(&a[4]),
                s(&a[5]),
                screen(6),
            ));
        }
        "GoldenIMFetchQuery" => {
            return text(params::im_fetch_query(
                s(&a[0]),
                s(&a[1]),
                s(&a[2]),
                s(&a[3]),
                screen(4),
            ));
        }
        "GoldenRoomEnterQuery" => {
            let room = if a[4] == true { s(&a[1]) } else { "" };
            return text(params::room_enter_query(
                s(&a[0]),
                room,
                s(&a[2]),
                s(&a[3]),
                screen(5),
            ));
        }
        other => return Err(format!("no Rust function for {other}")),
    };
    run(a)
}

fn replay(cases: &str) -> usize {
    let mut failures = Vec::new();
    let mut count = 0;
    for line in cases.lines().filter(|l| !l.trim().is_empty()) {
        let case: Value = serde_json::from_str(line).unwrap();
        let name = s(&case["fn"]);
        let args = case["args"].as_array().unwrap();
        let got = call(name, args);
        let ok = match (&got, case.get("ok"), case.get("error")) {
            (Ok(v), Some(want), _) => v == want,
            (Err(e), None, Some(want)) => e == s(want),
            _ => false,
        };
        if !ok {
            failures.push(format!("{name}({args:?}): want {line}, got {got:?}"));
        }
        count += 1;
    }
    assert!(
        failures.is_empty(),
        "{} of {count} cases differ:\n{}",
        failures.len(),
        failures[..failures.len().min(5)].join("\n")
    );
    count
}

#[test]
fn websocket_signature_matches_go() {
    assert_eq!(replay(include_str!("recorded/webcastsign.jsonl")), 235);
}

#[test]
fn a_bogus_and_sm3_match_go() {
    assert_eq!(replay(include_str!("recorded/ab_sign.jsonl")), 764);
}

#[test]
fn query_strings_and_page_parsing_match_go() {
    assert_eq!(replay(include_str!("recorded/params_page.jsonl")), 3007);
}
