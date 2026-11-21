use crate::{
    audio::checked,
    config::{Settings, key},
};
use anyhow::{Context, Result, ensure};
use futures_util::StreamExt;
use serde_json::{Value, json};
use std::time::Duration;
use tokio::sync::mpsc::Sender;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PromptKind {
    Chat,
    AmbientWelcome,
    JoinWelcome,
    GiftThanks,
    PostSongThanks,
}

fn qa_blocks(raw: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut cur = Vec::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        let starts_q = trimmed.starts_with("Q：")
            || trimmed.starts_with("Q:")
            || trimmed.starts_with("q：")
            || trimmed.starts_with("q:")
            || trimmed.starts_with("问：")
            || trimmed.starts_with("问:");
        if trimmed.is_empty() {
            if !cur.is_empty() {
                blocks.push(cur.join("\n"));
                cur.clear();
            }
            continue;
        }
        if starts_q && !cur.is_empty() {
            blocks.push(cur.join("\n"));
            cur.clear();
        }
        cur.push(trimmed.to_owned());
    }
    if !cur.is_empty() {
        blocks.push(cur.join("\n"));
    }
    blocks
}

fn kind_keywords(kind: PromptKind) -> &'static [&'static str] {
    match kind {
        PromptKind::AmbientWelcome => &[
            "冷场", "欢迎", "打招呼", "安静", "哈喽", "进直播间", "welcome", "quiet", "greet",
        ],
        PromptKind::JoinWelcome => &[
            "进入直播间",
            "新观众",
            "进房",
            "欢迎某某",
            "打招呼",
            "joined",
            "join",
            "enter",
            "greet",
        ],
        PromptKind::GiftThanks => &[
            "礼物", "感谢", "小心心", "玫瑰", "棒棒糖", "跑车", "火箭", "嘉年华", "投喂", "gift",
            "thank",
        ],
        PromptKind::PostSongThanks => &[
            "好听", "唱歌", "礼物", "感谢", "歌", "唱", "song", "sing", "gift", "thank",
        ],
        PromptKind::Chat => &[],
    }
}

fn score_block(block: &str, kind: PromptKind, input: &str) -> usize {
    let lower_block = block.to_lowercase();
    let mut score = 0usize;
    for kw in kind_keywords(kind) {
        if lower_block.contains(&kw.to_lowercase()) {
            score += 6;
        }
    }
    let lower_in = input.to_lowercase();
    // English word overlap
    for word in lower_in
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| w.len() >= 3)
    {
        if lower_block.contains(word) {
            score += 3;
        }
    }
    // CJK bigram overlap
    let cjk: Vec<char> = input
        .chars()
        .filter(|c| matches!(*c as u32, 0x3400..=0x9fff | 0xf900..=0xfaff))
        .collect();
    for pair in cjk.windows(2) {
        let mut s = String::with_capacity(8);
        s.push(pair[0]);
        s.push(pair[1]);
        if block.contains(&s) {
            score += 2;
        }
    }
    score
}

pub fn select_qa_reference(raw: &str, kind: PromptKind, input: &str, max_pairs: usize) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let blocks = qa_blocks(trimmed);
    if blocks.len() <= max_pairs || trimmed.chars().count() <= 900 {
        return trimmed.to_owned();
    }
    let mut scored: Vec<(usize, usize)> = blocks
        .iter()
        .enumerate()
        .map(|(idx, b)| (idx, score_block(b, kind, input)))
        .collect();
    scored.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let mut chosen: Vec<usize> = scored
        .into_iter()
        .filter(|(_, s)| *s > 0)
        .take(max_pairs)
        .map(|(i, _)| i)
        .collect();
    if chosen.is_empty() {
        chosen = (0..blocks.len().min(max_pairs.min(2))).collect();
    }
    chosen.sort_unstable();
    chosen
        .into_iter()
        .map(|i| blocks[i].as_str())
        .collect::<Vec<_>>()
        .join("\n\n")
}

pub fn build_system_prompt(c: &Settings, kind: PromptKind, input: &str) -> String {
    let mut parts = Vec::new();
    let persona = c.persona.trim();
    if !persona.is_empty() {
        parts.push(persona.to_owned());
    }
    if c.is_english() {
        parts.push(
            "This is an English livestream. Reply strictly in natural spoken English only, with no Chinese characters, no stage directions, no brackets, and no emojis. Output only the spoken line."
                .to_owned(),
        );
    } else {
        parts.push(
            "当前为中文直播间，只能使用自然口语化的简体中文回复，不要包含任何英文单词或英文字母，不要输出舞台动作描写、括号说明或表情符号，直接输出要说出来的正文。"
                .to_owned(),
        );
    }
    let max_pairs = match kind {
        PromptKind::GiftThanks | PromptKind::PostSongThanks => 5,
        _ => 4,
    };
    let qa = select_qa_reference(c.active_qa_reference(), kind, input, max_pairs);
    if !qa.is_empty() {
        if c.is_english() {
            parts.push(format!(
                "[QA Reference - follow this tone and response style naturally]\n{qa}"
            ));
        } else {
            parts.push(format!(
                "【QA 提示词参考（请参考以下话术风格与规则自然回应）】\n{qa}"
            ));
        }
    }
    parts.join("\n\n")
}

pub fn split_ready(text: &mut String, flush: bool) -> Option<String> {
    let mut chars = 0;
    let mut end = None;
    for (i, ch) in text.char_indices() {
        chars += 1;
        if "。！？!?\n".contains(ch) || (chars >= 16 && "，、,.；;".contains(ch)) || chars >= 85
        {
            end = Some(i + ch.len_utf8());
            break;
        }
    }
    let end = end.or((flush && !text.is_empty()).then_some(text.len()))?;
    Some(text.drain(..end).collect::<String>().trim().to_owned())
}
async fn emit(pending: &mut String, flush: bool, tx: &Sender<String>) -> Result<()> {
    while let Some(s) = split_ready(pending, flush) {
        if !s.is_empty() {
            tx.send(s).await.context("本次回复已结束")?;
        }
    }
    Ok(())
}
async fn event(
    line: &[u8],
    pending: &mut String,
    tx: &Sender<String>,
    total: &mut usize,
) -> Result<bool> {
    let line = std::str::from_utf8(line)?.trim();
    let Some(data) = line.strip_prefix("data:").map(str::trim) else {
        return Ok(false);
    };
    if data == "[DONE]" {
        return Ok(true);
    }
    let v: Value = serde_json::from_str(data).context("语言模型返回了无效的流式数据")?;
    ensure!(v.get("error").is_none(), "语言模型中止了回复");
    if let Some(s) = v
        .pointer("/choices/0/delta/content")
        .and_then(Value::as_str)
    {
        *total += s.len();
        ensure!(*total <= 12_000, "单次模型回复超出长度限制");
        pending.push_str(s);
        emit(pending, false, tx).await?;
    }
    Ok(v.pointer("/choices/0/finish_reason")
        .is_some_and(|r| r.is_string()))
}
pub async fn sentences(
    http: &reqwest::Client,
    c: &Settings,
    kind: PromptKind,
    input: &str,
    history: &[(String, String)],
    tx: Sender<String>,
) -> Result<()> {
    ensure!(
        !c.llm_endpoint.is_empty() && !c.llm_model.trim().is_empty(),
        "请在设置中填写语言模型接口与模型名"
    );
    let mut messages = Vec::new();
    let system = build_system_prompt(c, kind, input);
    if !system.is_empty() {
        messages.push(json!({"role":"system","content":system}));
    }
    for (prompt, reply) in history {
        messages.push(json!({"role":"user","content":prompt}));
        messages.push(json!({"role":"assistant","content":reply}));
    }
    messages.push(json!({"role":"user","content":input}));
    let max_tokens = match kind {
        PromptKind::Chat => 360,
        PromptKind::AmbientWelcome | PromptKind::JoinWelcome => 100,
        PromptKind::GiftThanks | PromptKind::PostSongThanks => 180,
    };
    let mut req = http.post(&c.llm_endpoint).timeout(Duration::from_secs(90));
    if let Some(k) = key("llm")? {
        req = req.bearer_auth(k);
    }
    let resp = req
        .json(&json!({"model":c.llm_model,"messages":messages,"stream":true,"max_tokens":max_tokens}))
        .send()
        .await
        .map_err(|e| e.without_url())?;
    let mut stream = checked(resp, "语言模型").await?.bytes_stream();
    let mut buf = Vec::new();
    let mut pending = String::new();
    let mut total = 0;
    let mut finished = false;
    'outer: while let Some(chunk) = stream.next().await {
        buf.extend_from_slice(&chunk.map_err(|e| e.without_url())?);
        ensure!(buf.len() < 256_000, "模型事件过大");
        while let Some(i) = buf.iter().position(|b| *b == b'\n') {
            let line = buf.drain(..=i).collect::<Vec<_>>();
            if event(&line, &mut pending, &tx, &mut total).await? {
                finished = true;
                break 'outer;
            }
        }
    }
    if !finished && !buf.is_empty() {
        finished = event(&buf, &mut pending, &tx, &mut total).await?;
    }
    ensure!(finished, "语言模型连接意外中断");
    ensure!(total > 0, "语言模型未返回可朗读的文字");
    emit(&mut pending, true, &tx).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streamed_chinese_keeps_every_character() {
        let original = Settings::default().persona;
        let mut pending = String::new();
        let mut spoken = String::new();
        for ch in original.chars() {
            pending.push(ch);
            if let Some(line) = split_ready(&mut pending, false) {
                spoken.push_str(&line);
            }
        }
        while let Some(line) = split_ready(&mut pending, true) {
            spoken.push_str(&line);
        }
        assert_eq!(spoken, original);
        assert!(pending.is_empty());
    }

    #[test]
    fn attaches_relevant_qa_and_language_guard() {
        let cfg = Settings::default();
        let join_sys = build_system_prompt(&cfg, PromptKind::JoinWelcome, "观众小明进入直播间");
        assert!(join_sys.contains("当前为中文直播间"));
        assert!(join_sys.contains("欢迎某某"));

        let gift_sys = build_system_prompt(&cfg, PromptKind::PostSongThanks, "唱完歌收到玫瑰和跑车");
        assert!(gift_sys.contains("玫瑰") || gift_sys.contains("礼物"));

        let mut en = Settings::default();
        en.live_language = "en".into();
        let en_sys = build_system_prompt(&en, PromptKind::JoinWelcome, "Viewer Alice joined");
        assert!(en_sys.contains("English livestream"));
        assert!(en_sys.contains("QA Reference"));
    }
}
