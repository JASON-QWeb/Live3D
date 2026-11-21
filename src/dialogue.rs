use crate::{
    audio::checked,
    config::{Settings, key},
};
use anyhow::{Context, Result, ensure};
use futures_util::StreamExt;
use serde_json::{Value, json};
use std::time::Duration;
use tokio::sync::mpsc::Sender;

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
    input: &str,
    history: &[(String, String)],
    tx: Sender<String>,
) -> Result<()> {
    ensure!(
        !c.llm_endpoint.is_empty() && !c.llm_model.trim().is_empty(),
        "请在设置中填写语言模型接口与模型名"
    );
    let mut messages = Vec::new();
    if !c.persona.trim().is_empty() {
        messages.push(json!({"role":"system","content":c.persona}));
    }
    for (prompt, reply) in history {
        messages.push(json!({"role":"user","content":prompt}));
        messages.push(json!({"role":"assistant","content":reply}));
    }
    messages.push(json!({"role":"user","content":input}));
    let mut req = http.post(&c.llm_endpoint).timeout(Duration::from_secs(90));
    if let Some(k) = key("llm")? {
        req = req.bearer_auth(k);
    }
    let resp = req
        .json(&json!({"model":c.llm_model,"messages":messages,"stream":true,"max_tokens":400}))
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
}
