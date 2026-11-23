use crate::{
    api::Problem,
    config,
    library::{BgmTrack, bgm_list},
    music::{self, Provider},
    output,
    state::{App, Screen, lock},
};
use anyhow::{Context, Result, ensure};
use axum::{Json, extract::State};
use serde::Deserialize;
use serde_json::{Value, json};

pub fn notify(app: &App) {
    app.notify_stage(json!({"event":"bgm_state","bgm":output::bgm_snapshot(app)}));
}

impl Screen {
    pub fn advance_bgm(&mut self, items: &[BgmTrack]) {
        if self.requested_bgm.take().is_none() && !items.is_empty() {
            self.bgm_index = (self.bgm_index + 1) % items.len();
        }
        self.requested_bgm = self.bgm_requests.pop_front();
        if self.requested_bgm.is_none() {
            if let Some(index) = (0..items.len())
                .map(|n| (self.bgm_index + n) % items.len())
                .find(|&i| !self.failed_bgm.contains(&items[i].id))
            {
                self.bgm_index = index;
            } else {
                self.bgm_enabled = false;
                self.failed_bgm.clear();
            }
        }
        self.bgm_revision = self.bgm_revision.wrapping_add(1);
    }
}

pub fn enqueue(app: &App, mut item: BgmTrack) -> Result<()> {
    item.online.as_ref().context("请选择在线歌曲")?.validate()?;
    ensure!(
        !item.title.trim().is_empty() && item.title.len() <= 2048,
        "歌曲名称无效"
    );
    item.id = uuid::Uuid::new_v4();
    item.source_ext.clear();
    let has_rotation = !bgm_list(app)?.is_empty();
    let mut s = lock(&app.screen);
    ensure!(s.bgm_requests.len() < 50, "点歌队列已满");
    if (!s.bgm_enabled || !has_rotation) && s.requested_bgm.is_none() {
        s.requested_bgm = Some(item);
        s.bgm_revision = s.bgm_revision.wrapping_add(1);
    } else {
        s.bgm_requests.push_back(item);
    }
    s.bgm_enabled = true;
    drop(s);
    notify(app);
    Ok(())
}

#[derive(Deserialize)]
pub struct Lookup {
    provider: Provider,
    input: String,
}
pub async fn search(
    State(app): State<App>,
    Json(q): Json<Lookup>,
) -> Result<Json<Vec<BgmTrack>>, Problem> {
    Ok(Json(music::search(&app, q.provider, &q.input).await?))
}

pub async fn import(State(app): State<App>, Json(q): Json<Lookup>) -> Result<Json<Value>, Problem> {
    let tracks = music::playlist(&app, q.provider, &q.input).await?;
    let _guard = app.library_gate.lock().await;
    let mut items = bgm_list(&app)?;
    let original = items.len();
    for track in tracks {
        if !items.iter().any(|t| t.online == track.online) {
            items.push(track);
            if items.len() > 500 {
                return Err(anyhow::anyhow!("导入后超过背景音乐 500 首上限").into());
            }
        }
    }
    config::write_json(&app.dir.join("bgm.json"), &items)?;
    {
        let mut s = lock(&app.screen);
        if !s.bgm_enabled && original < items.len() && s.requested_bgm.is_none() {
            s.bgm_index = original;
            s.bgm_enabled = true;
            s.failed_bgm.clear();
            s.bgm_revision = s.bgm_revision.wrapping_add(1);
        }
    }
    notify(&app);
    Ok(Json(json!({"added":items.len()-original})))
}

pub async fn request(
    State(app): State<App>,
    Json(track): Json<BgmTrack>,
) -> Result<Json<Value>, Problem> {
    enqueue(&app, track)?;
    Ok(Json(json!({"ok":true})))
}

pub fn audience_request(app: &App, title: &str) -> Result<()> {
    let run = app.active()?;
    let permit = app
        .music_lookups
        .clone()
        .try_acquire_owned()
        .context("在线点歌请求繁忙")?;
    let (provider, term) = title
        .strip_prefix("qq ")
        .map(|term| (Provider::Qq, term))
        .unwrap_or((
            Provider::Netease,
            title.strip_prefix("wy ").unwrap_or(title),
        ));
    let app = app.clone();
    let term = term.to_owned();
    tokio::spawn(async move {
        let _permit = permit;
        let result = tokio::select! {
            biased;
            _ = run.stop.cancelled() => return,
            result = music::search(&app,provider,&term) => result,
        };
        let result = result.and_then(|items| {
            let track = items.into_iter().next().context("找不到匹配的在线歌曲")?;
            let title = track.title.clone();
            enqueue(&app, track)?;
            Ok(title)
        });
        match result {
            Ok(title) => app.event("背景音乐点歌", title),
            Err(e) => app.event("背景音乐点歌", format!("{term} · {e}")),
        }
    });
    Ok(())
}
