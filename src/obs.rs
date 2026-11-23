use crate::state::App;
use anyhow::{Context, Result, bail, ensure};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::time::Duration;
use tokio::net::TcpStream;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async, tungstenite::Message};
use uuid::Uuid;

const SCENE: &str = "Avatar3D · 数字人";
const SOURCE: &str = "Avatar3D · 舞台输出";
const BUNDLE: &str = "com.obsproject.obs-studio";

/// Whether OBS Studio is in an Applications folder or known to Spotlight.
pub async fn installed() -> bool {
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    let folders = [
        Some(std::path::PathBuf::from("/Applications")),
        home.map(|h| h.join("Applications")),
    ];
    if folders.iter().flatten().any(|d| d.join("OBS.app").is_dir()) {
        return true;
    }
    let query = tokio::process::Command::new("/usr/bin/mdfind")
        .arg(format!("kMDItemCFBundleIdentifier == '{BUNDLE}'"))
        .kill_on_drop(true)
        .output();
    matches!(
        tokio::time::timeout(Duration::from_secs(3), query).await,
        Ok(Ok(out)) if out.status.success() && !out.stdout.trim_ascii().is_empty()
    )
}

/// Opens OBS Studio, or brings it to the front when it is already running.
pub async fn open() -> Result<()> {
    let out = tokio::process::Command::new("/usr/bin/open")
        .args(["-b", BUNDLE])
        .kill_on_drop(true)
        .output();
    let out = tokio::time::timeout(Duration::from_secs(15), out)
        .await
        .context("打开 OBS 超时")?
        .context("无法调用系统打开 OBS")?;
    ensure!(
        out.status.success(),
        "这台 Mac 上没有找到 OBS Studio，请先安装"
    );
    Ok(())
}
type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;
async fn receive(ws: &mut Socket) -> Result<Value> {
    loop {
        match ws.next().await.context("OBS 已断开")?? {
            Message::Text(s) => return Ok(serde_json::from_str(&s)?),
            Message::Close(_) => bail!("OBS 已关闭连接，请核对 WebSocket 密码"),
            Message::Ping(_) => ws.flush().await?,
            _ => (),
        }
    }
}
async fn connect(port: u16, password: &str) -> Result<Socket> {
    let (mut ws, _) = connect_async(format!("ws://127.0.0.1:{port}"))
        .await
        .context("无法连接本机 OBS WebSocket 服务")?;
    let hello = receive(&mut ws).await?;
    ensure!(hello["op"] == 0, "OBS 未返回握手信息");
    let mut d = json!({"rpcVersion":1,"eventSubscriptions":0});
    if let Some(auth) = hello["d"].get("authentication") {
        let salt = auth["salt"].as_str().context("OBS 认证数据无效")?;
        let challenge = auth["challenge"].as_str().context("OBS 认证数据无效")?;
        ensure!(!password.is_empty(), "请填写 OBS WebSocket 服务器密码");
        let secret = STANDARD.encode(Sha256::digest(format!("{password}{salt}")));
        d["authentication"] =
            json!(STANDARD.encode(Sha256::digest(format!("{secret}{challenge}"))));
    }
    ws.send(Message::text(json!({"op":1,"d":d}).to_string()))
        .await?;
    ensure!(receive(&mut ws).await?["op"] == 2, "OBS 认证失败");
    Ok(ws)
}
async fn call(ws: &mut Socket, name: &str, data: Value) -> Result<Value> {
    let id = Uuid::new_v4().to_string();
    ws.send(Message::text(
        json!({"op":6,"d":{"requestType":name,"requestId":id,"requestData":data}}).to_string(),
    ))
    .await?;
    loop {
        let r = receive(ws).await?;
        if r["op"] != 7 || r["d"]["requestId"] != id {
            continue;
        }
        let s = &r["d"]["requestStatus"];
        let comment = s["comment"]
            .as_str()
            .map(|c| format!("：{c}"))
            .unwrap_or_default();
        ensure!(
            s["result"] == true,
            "OBS {name} 请求失败（{}{comment}）",
            s["code"]
        );
        return Ok(r["d"]["responseData"].clone());
    }
}
fn named(list: &Value, collection: &str, field: &str, name: &str) -> bool {
    list[collection]
        .as_array()
        .is_some_and(|arr| arr.iter().any(|v| v[field] == name))
}
async fn configure(ws: &mut Socket, url: &str) -> Result<Value> {
    let existing = call(ws, "GetInputList", json!({})).await?;
    let has_source = named(&existing, "inputs", "inputName", SOURCE);
    if has_source {
        let input = call(ws, "GetInputSettings", json!({"inputName":SOURCE})).await?;
        ensure!(
            input["inputKind"] == "browser_source" && input["inputSettings"]["url"] == url,
            "OBS 已有同名输入，且指向其他来源"
        );
    }
    let list = call(ws, "GetSceneList", json!({})).await?;
    let has_scene = named(&list, "scenes", "sceneName", SCENE);
    if has_scene {
        let items = call(ws, "GetSceneItemList", json!({"sceneName":SCENE})).await?;
        if has_source && named(&items, "sceneItems", "sourceName", SOURCE) {
            return Ok(json!({"scene":SCENE,"created":false}));
        }
        ensure!(
            items["sceneItems"].as_array().is_some_and(Vec::is_empty),
            "OBS 已有同名场景，内含其他来源"
        );
    } else {
        call(ws, "CreateScene", json!({"sceneName":SCENE})).await?;
    }
    if has_source {
        call(
            ws,
            "CreateSceneItem",
            json!({"sceneName":SCENE,"sourceName":SOURCE,"sceneItemEnabled":true}),
        )
        .await?;
    } else {
        let size = call(ws, "GetVideoSettings", json!({})).await?;
        let input = json!({
            "sceneName": SCENE, "inputName": SOURCE, "inputKind": "browser_source",
            "sceneItemEnabled": true,
            "inputSettings": {
                "url": url, "width": size["baseWidth"], "height": size["baseHeight"],
                "reroute_audio": true, "shutdown": true, "restart_when_active": false,
                "fps": 30, "css": ""
            }
        });
        call(ws, "CreateInput", input).await?;
    }
    call(
        ws,
        "GetSceneItemId",
        json!({"sceneName":SCENE,"sourceName":SOURCE}),
    )
    .await?;
    Ok(json!({"scene":SCENE,"created":true}))
}
/// Runs `op` on one authenticated OBS connection, one request group at a time.
async fn with_obs<T>(
    app: &App,
    port: u16,
    password: &str,
    op: impl AsyncFnOnce(&mut Socket) -> Result<T>,
) -> Result<T> {
    ensure!(port > 0 && password.len() < 2048, "OBS 连接参数无效");
    let _guard = app.obs_gate.lock().await;
    let run = async {
        let mut ws = connect(port, password).await?;
        let result = op(&mut ws).await;
        let _ = ws.close(None).await;
        result
    };
    tokio::time::timeout(Duration::from_secs(12), run)
        .await
        .context("本机 OBS 响应超时")?
}
/// The stage source plays on this Mac's audio output as well as into OBS's
/// stream and recording, so apps that capture system audio, such as
/// 抖音直播伴侣 on macOS, hear the avatar. OBS's virtual camera carries no audio.
const STAGE_MONITOR: &str = "OBS_MONITORING_TYPE_MONITOR_AND_OUTPUT";

pub async fn setup(app: &App, port: u16, password: &str, host: &str) -> Result<Value> {
    let url = format!("http://{host}/stage#{}", app.tokens.stage);
    with_obs(app, port, password, async |ws| {
        let result = configure(ws, &url).await?;
        // Applied every time, so a scene set up before this existed is fixed too.
        call(
            ws,
            "SetInputAudioMonitorType",
            json!({"inputName":SOURCE,"monitorType":STAGE_MONITOR}),
        )
        .await?;
        Ok(result)
    })
    .await
}

/// Streaming state, destination and program scene. The stream key never leaves the server.
async fn stream_state(ws: &mut Socket) -> Result<Value> {
    let out = call(ws, "GetStreamStatus", json!({})).await?;
    let service = call(ws, "GetStreamServiceSettings", json!({})).await?;
    let program = call(ws, "GetCurrentProgramScene", json!({})).await?;
    let settings = &service["streamServiceSettings"];
    let custom = service["streamServiceType"] == "rtmp_custom";
    let scene = program["currentProgramSceneName"].as_str().unwrap_or("");
    Ok(json!({
        "live": out["outputActive"] == true,
        "reconnecting": out["outputReconnecting"] == true,
        "duration_ms": out["outputDuration"].as_u64().unwrap_or(0),
        "skipped_frames": out["outputSkippedFrames"].as_u64().unwrap_or(0),
        "total_frames": out["outputTotalFrames"].as_u64().unwrap_or(0),
        "custom": custom,
        "server": if custom { settings["server"].as_str().unwrap_or("") } else { "" },
        "service": if custom { "" } else { settings["service"].as_str().unwrap_or("") },
        "key_set": settings["key"].as_str().is_some_and(|k| !k.trim().is_empty()),
        "scene": scene,
        "avatar_scene": scene == SCENE,
    }))
}
pub async fn stream_status(app: &App, port: u16, password: &str) -> Result<Value> {
    with_obs(app, port, password, async |ws| stream_state(ws).await).await
}

fn check_stream_server(server: &str) -> Result<()> {
    let parsed = url::Url::parse(server).ok();
    ensure!(
        server.len() <= 2048
            && parsed.as_ref().is_some_and(|u| {
                ["rtmp", "rtmps", "srt", "rist"].contains(&u.scheme()) && u.host().is_some()
            }),
        "推流地址要以 rtmp://、rtmps:// 或 srt:// 开头"
    );
    Ok(())
}

fn saved_stream_key(service: &Value, server: &str) -> Result<String> {
    let settings = &service["streamServiceSettings"];
    ensure!(
        service["streamServiceType"] == "rtmp_custom"
            && settings["server"].as_str() == Some(server),
        "请为此推流地址填写推流码"
    );
    settings["key"]
        .as_str()
        .filter(|key| !key.trim().is_empty())
        .map(str::to_owned)
        .context("请填写推流码")
}

/// OBS stores the destination and its key together in the current profile.
pub async fn set_stream(
    app: &App,
    port: u16,
    password: &str,
    server: &str,
    key: &str,
) -> Result<Value> {
    let (server, key) = (server.trim(), key.trim());
    check_stream_server(server)?;
    ensure!(
        key.len() <= 4096 && !key.chars().any(char::is_control),
        "推流码格式无效"
    );
    with_obs(app, port, password, async |ws| {
        let out = call(ws, "GetStreamStatus", json!({})).await?;
        ensure!(
            out["outputActive"] != true,
            "正在直播，先结束直播再改推流设置"
        );
        let key = if key.is_empty() {
            let current = call(ws, "GetStreamServiceSettings", json!({})).await?;
            saved_stream_key(&current, server)?
        } else {
            key.to_owned()
        };
        call(
            ws,
            "SetStreamServiceSettings",
            json!({
                "streamServiceType": "rtmp_custom",
                "streamServiceSettings": {"server": server, "key": key},
            }),
        )
        .await?;
        stream_state(ws).await
    })
    .await
}

/// Starts or stops the OBS stream output, then waits briefly for OBS to report the change.
pub async fn set_live(app: &App, port: u16, password: &str, live: bool) -> Result<Value> {
    with_obs(app, port, password, async |ws| {
        let state = stream_state(ws).await?;
        if state["live"] == live {
            return Ok(state);
        }
        if live {
            ensure!(
                state["key_set"] == true,
                "OBS 还没有推流码，请先填写推流地址和推流码，点「保存到 OBS」"
            );
            call(ws, "StartStream", json!({})).await?;
        } else {
            call(ws, "StopStream", json!({})).await?;
        }
        for _ in 0..10 {
            tokio::time::sleep(Duration::from_millis(300)).await;
            let state = stream_state(ws).await?;
            if state["live"] == live {
                return Ok(state);
            }
        }
        stream_state(ws).await
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::{check_stream_server, saved_stream_key};
    use serde_json::json;

    #[test]
    fn keeps_a_stream_key_only_for_its_existing_custom_destination() {
        let server = "rtmps://a.rtmps.youtube.com:443/live2";
        let key = uuid::Uuid::new_v4().to_string();
        let mut service = json!({
            "streamServiceType": "rtmp_custom",
            "streamServiceSettings": { "server": server, "key": key }
        });
        assert_eq!(saved_stream_key(&service, server).unwrap(), key);
        for changed in [
            "rtmp://push-rtmp-l1.douyincdn.com/stage",
            "rtmps://a.rtmps.youtube.com:443/",
            "rtmp://a.rtmps.youtube.com:443/live2",
        ] {
            assert!(saved_stream_key(&service, changed).is_err());
        }
        service["streamServiceType"] = json!("rtmp_common");
        assert!(saved_stream_key(&service, server).is_err());
        service["streamServiceType"] = json!("rtmp_custom");
        service["streamServiceSettings"]["key"] = json!("");
        assert!(saved_stream_key(&service, server).is_err());
    }

    #[test]
    fn accepts_push_servers_and_rejects_other_urls() {
        for ok in [
            "rtmp://push-rtmp-l1.douyincdn.com/stage",
            "rtmps://a.rtmps.youtube.com:443/live2",
            "srt://live.example.com:9000?streamid=abc",
        ] {
            assert!(check_stream_server(ok).is_ok(), "{ok}");
        }
        for bad in [
            "",
            "push.example.com/live",
            "https://example.com/live",
            "rtmp://",
        ] {
            assert!(check_stream_server(bad).is_err(), "{bad}");
        }
    }
}
