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
        ensure!(s["result"] == true, "OBS {name} 请求失败（{}）", s["code"]);
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
pub async fn setup(app: &App, port: u16, password: &str, host: &str) -> Result<Value> {
    ensure!(port > 0 && password.len() < 2048, "OBS 连接参数无效");
    let _guard = app.obs_gate.lock().await;
    let url = format!("http://{host}/stage#{}", app.tokens.stage);
    let op = async {
        let mut ws = connect(port, password).await?;
        let result = configure(&mut ws, &url).await;
        let _ = ws.close(None).await;
        result
    };
    tokio::time::timeout(Duration::from_secs(12), op)
        .await
        .context("本机 OBS 响应超时")?
}
