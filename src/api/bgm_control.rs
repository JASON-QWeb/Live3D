use super::{Api, S};
use crate::{output, state::lock};
use anyhow::Context;
use axum::{Json, extract::State};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct BgmControl {
    action: String,
    #[serde(default)]
    id: Option<uuid::Uuid>,
}
pub(super) async fn bgm_control(State(app): S, Json(q): Json<BgmControl>) -> Api<Json<Value>> {
    let items = crate::library::bgm_list(&app)?;
    {
        let mut s = lock(&app.screen);
        let available = !items.is_empty() || s.requested_bgm.is_some();
        match q.action.as_str() {
            "toggle" | "start" | "stop" => {
                let enable = q.action == "start" || (q.action == "toggle" && !s.bgm_enabled);
                if enable && !available {
                    return Err(anyhow::anyhow!("请先导入背景音乐").into());
                }
                s.bgm_enabled = enable;
                if enable {
                    s.failed_bgm.clear();
                }
                s.bgm_revision = s.bgm_revision.wrapping_add(1);
            }
            "next" => {
                if !available {
                    return Err(anyhow::anyhow!("背景音乐列表为空").into());
                }
                s.advance_bgm(&items);
            }
            "select" => {
                let id = q.id.context("请选择要播放的背景音乐")?;
                s.bgm_index = items
                    .iter()
                    .position(|t| t.id == id)
                    .context("找不到该背景音乐")?;
                s.requested_bgm = None;
                s.failed_bgm.clear();
                s.bgm_enabled = true;
                s.bgm_revision = s.bgm_revision.wrapping_add(1);
            }
            _ => return Err(anyhow::anyhow!("未知背景音乐操作").into()),
        }
    }
    let bgm = output::bgm_snapshot(&app);
    app.notify_stage(json!({"event": "bgm_state", "bgm": bgm}));
    Ok(Json(json!({"ok": true, "bgm": bgm})))
}

#[derive(Deserialize)]
pub(super) struct StageClient {
    revision: Option<u64>,
    failed: Option<uuid::Uuid>,
    client: String,
}
pub(super) async fn stage_bgm_next(State(app): S, Json(q): Json<StageClient>) -> Api<Json<Value>> {
    let items = crate::library::bgm_list(&app)?;
    let is_owner = {
        let mut s = lock(&app.screen);
        if s.lease.is_empty()
            || s.lease != q.client
            || q.revision.is_some_and(|r| r != s.bgm_revision)
        {
            false
        } else {
            if s.bgm_enabled {
                if let Some(id) = q.failed {
                    if items.iter().any(|t| t.id == id) {
                        s.failed_bgm.insert(id);
                    }
                    app.event("背景音乐", "在线曲目播放失败，已跳过");
                } else {
                    s.failed_bgm.clear();
                }
                s.advance_bgm(&items);
            }
            true
        }
    };
    if !is_owner {
        return Ok(Json(json!({"ok": false})));
    }
    let bgm = output::bgm_snapshot(&app);
    app.notify_stage(json!({"event": "bgm_state", "bgm": bgm}));
    Ok(Json(json!({"ok": true, "bgm": bgm})))
}
