use crate::{
    config::now,
    session::Run,
    state::{App, lock},
};
use anyhow::{Context, Result, bail, ensure};
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
use tokio::{
    net::TcpStream,
    sync::{mpsc, watch},
    time::{Instant, timeout},
};
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream, connect_async,
    tungstenite::{Message, client::IntoClientRequest},
};
use uuid::Uuid;

pub enum Command {
    Signal(u8, Value),
    Audio(Bytes),
    Line(String, u64),
}
pub struct Packet {
    pub epoch: u64,
    pub cmd: Command,
}
type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;
struct Link {
    ws: Ws,
    id: String,
    conn: String,
    seq: u64,
}
impl Link {
    async fn open(id: &str, key: &str) -> Result<Self> {
        let conn = Uuid::new_v4().to_string();
        let mut req =
            format!("wss://api.vidu.cn/live/v1/external-lives/{id}/stream?conn_id={conn}")
                .into_client_request()?;
        req.headers_mut()
            .insert("Authorization", format!("Token {key}").parse()?);
        let (ws, _) = timeout(Duration::from_secs(8), connect_async(req)).await??;
        Ok(Self {
            ws,
            id: id.to_owned(),
            conn,
            seq: 0,
        })
    }
    async fn write(&mut self, m: Message) -> Result<()> {
        timeout(Duration::from_secs(4), self.ws.send(m)).await??;
        Ok(())
    }
    async fn signal(&mut self, t: u8, p: Value) -> Result<()> {
        self.seq += 1;
        let msg =
            json!({"type":t,"live_id":self.id,"conn_id":self.conn,"seq_id":self.seq,"payload":p});
        self.write(Message::Text(msg.to_string().into())).await
    }
    async fn hangup(&mut self) -> Result<()> {
        self.signal(5, json!({"hangup":{"hangup_reason":"client_hangup"}}))
            .await?;
        timeout(Duration::from_secs(2), self.ws.close(None)).await??;
        Ok(())
    }
    async fn clear_stale_audio(&mut self, epoch: &mut watch::Receiver<u64>) -> Result<()> {
        // A replacement socket subscribes after the old stream was interrupted.
        // The watch value survives reconnection even when changed() is already caught up.
        if *epoch.borrow_and_update() != 0 {
            self.signal(7, json!({})).await?;
        }
        Ok(())
    }
}
pub async fn request_end(id: &str, key: &str) -> Result<()> {
    let mut l = Link::open(id, key).await?;
    l.signal(1, json!({"conn_init":{"version":1}})).await?;
    l.hangup().await
}
pub fn text_msg(text: &str) -> Value {
    json!({"text_msg":{"msg_id":Uuid::new_v4().to_string(),"content":text,"timestamp":now()}})
}
enum Update {
    Ready,
    Retry,
    End,
    Other,
}
fn handle(app: &App, v: &Value) -> Result<Update> {
    let p = &v["payload"];
    match v["type"].as_u64() {
        Some(2) if p["conn_init_ack"]["success"] == true => Ok(Update::Ready),
        Some(2) if p["conn_init_ack"]["error_code"] == "NOT_READY" => Ok(Update::Retry),
        Some(2) if p["conn_init_ack"]["error_code"] == "LIVE_ENDED" => Ok(Update::End),
        Some(2) => bail!("Vidu 未接受会话初始化"),
        Some(6) => Ok(Update::End),
        Some(12) => {
            app.event(
                "云端",
                if p["prompt_operation_ack"]["success"] == true {
                    "参考图已接收，等待执行"
                } else {
                    "参考图请求被拒绝"
                },
            );
            Ok(Update::Other)
        }
        Some(103) => {
            app.event("云端", "Vidu 报告信令错误");
            Ok(Update::Other)
        }
        Some(104) => {
            let m = &p["msg"];
            let src = m["source"].as_str().unwrap_or("操作");
            let label = match src {
                "prompt_operation" => "参考图",
                "baseline_reset" => "姿态复位",
                "switch_avatar" => "形象切换",
                _ => "云端操作",
            };
            let state = if matches!(m["status"].as_str(), Some("preparing" | "running")) {
                "执行中"
            } else if m["success"] == true {
                "已完成"
            } else {
                "执行失败"
            };
            app.event("云端", format!("{label}{state}"));
            Ok(Update::Other)
        }
        _ => Ok(Update::Other),
    }
}
async fn incoming(l: &mut Link, app: &App, m: Message) -> Result<Update> {
    match m {
        Message::Text(s) => handle(app, &serde_json::from_str(&s)?),
        Message::Ping(_) => {
            timeout(Duration::from_secs(3), l.ws.flush()).await??;
            Ok(Update::Other)
        }
        Message::Close(Some(f)) if u16::from(f.code) == 4001 => Ok(Update::End),
        Message::Close(_) => bail!("云端关闭了信令连接"),
        _ => Ok(Update::Other),
    }
}
async fn packet(l: &mut Link, app: &App, p: Packet) -> Result<()> {
    match p.cmd {
        Command::Signal(t, v) => {
            l.signal(t, v).await?;
            if t == 9 {
                lock(&app.view).metrics.actions_sent += 1;
            }
        }
        Command::Audio(b) => {
            let n = b.len() as u64;
            l.write(Message::Binary(b)).await?;
            lock(&app.view).metrics.pcm_bytes += n;
        }
        Command::Line(s, ms) => {
            if !s.is_empty() {
                l.signal(10, text_msg(&s)).await?;
            }
            let mut v = lock(&app.view);
            v.subtitle = s;
            v.subtitle_until = now() + ms + 300;
        }
    }
    Ok(())
}
pub async fn serve(
    app: &App,
    run: &Arc<Run>,
    id: &str,
    key: &str,
    rx: &mut mpsc::Receiver<Packet>,
) -> Result<()> {
    let mut l = Link::open(id, key).await?;
    l.signal(1, json!({"conn_init":{"version":1}})).await?;
    let mut epoch = run.epoch.subscribe();
    let mut ready = false;
    let mut retry = None;
    let began = Instant::now();
    let mut last = Instant::now();
    let mut tick = tokio::time::interval(Duration::from_millis(500));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            biased;
            _ = run.stop.cancelled() => {
                l.hangup().await?;
                return Ok(());
            }
            _ = epoch.changed() => {
                if ready { l.signal(7, json!({})).await?; }
            }
            message = l.ws.next() => {
                let message = message.context("信令连接已断开")??;
                last = Instant::now();
                match incoming(&mut l, app, message).await? {
                    Update::Ready if !ready => {
                        l.clear_stale_audio(&mut epoch).await?;
                        ready = true;
                        retry = None;
                        let t = now();
                        let _ = run.last_activity_ms.compare_exchange(
                            0,
                            t,
                            std::sync::atomic::Ordering::SeqCst,
                            std::sync::atomic::Ordering::Relaxed,
                        );
                        let _ = run.last_song_end_ms.compare_exchange(
                            0,
                            t,
                            std::sync::atomic::Ordering::SeqCst,
                            std::sync::atomic::Ordering::Relaxed,
                        );
                        let _ = run.last_ambient_ms.compare_exchange(
                            0,
                            t,
                            std::sync::atomic::Ordering::SeqCst,
                            std::sync::atomic::Ordering::Relaxed,
                        );
                        app.phase("live", "云端会话已就绪");
                        app.event("连线", "数字人信令已就绪");
                    }
                    Update::Retry => retry = Some(Instant::now() + Duration::from_secs(1)),
                    Update::End => return Ok(()),
                    Update::Ready | Update::Other => (),
                }
            }
            _ = tick.tick() => {
                ensure!(last.elapsed() < Duration::from_secs(22), "云端心跳超时");
                ensure!(ready || began.elapsed() < Duration::from_secs(75), "云端初始化超时");
                if retry.is_some_and(|time| Instant::now() >= time) {
                    retry = None;
                    l.signal(1, json!({"conn_init":{"version":1}})).await?;
                }
            }
            next = rx.recv(), if ready => {
                let next = next.context("会话控制通道已关闭")?;
                if next.epoch == *run.epoch.borrow() {
                    packet(&mut l, app, next).await?;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;
    use tokio_tungstenite::accept_async;

    #[tokio::test]
    async fn replacement_socket_sends_the_interrupt_missed_while_disconnected() -> Result<()> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let address = listener.local_addr()?;
        let accept = async {
            let (stream, _) = listener.accept().await?;
            Ok::<_, anyhow::Error>(accept_async(stream).await?)
        };
        let connect = connect_async(format!("ws://{address}"));
        let (server, client) = tokio::join!(accept, connect);
        let mut server = server?;
        let mut link = Link {
            ws: client?.0,
            id: Uuid::new_v4().to_string(),
            conn: Uuid::new_v4().to_string(),
            seq: 0,
        };
        let (epoch, mut initial) = watch::channel(0);
        link.clear_stale_audio(&mut initial).await?;
        assert_eq!(link.seq, 0);

        epoch.send_modify(|value| *value += 1);
        let mut replacement = epoch.subscribe();
        assert!(!replacement.has_changed()?);
        link.clear_stale_audio(&mut replacement).await?;
        let frame = timeout(Duration::from_secs(2), server.next())
            .await?
            .unwrap()?;
        let signal: Value = serde_json::from_str(frame.to_text()?)?;
        assert_eq!(signal["type"], 7);
        assert_eq!(signal["seq_id"], 1);
        assert_eq!(signal["live_id"], link.id);
        assert_eq!(signal["conn_id"], link.conn);
        assert_eq!(signal["payload"], json!({}));
        Ok(())
    }
}
