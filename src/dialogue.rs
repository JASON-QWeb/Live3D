use crate::{
    audio::checked,
    config::{Settings, key},
};
use anyhow::{Context, Result, ensure};
use futures_util::StreamExt;
use serde::Serialize;
use serde_json::{Value, json};
use std::time::Duration;
use tokio::{sync::mpsc::Sender, time::Instant};

pub fn system_prompt(persona: &str, knowledge: &str, english: bool) -> String {
    let guard = if english {
        "Use spoken English only for this livestream. Output one or two short spoken sentences without Chinese characters, stage directions, brackets or emojis. Use the supplied facts and follow the reference rules. If a rule calls for silence or a factual answer lacks supporting information, output only no_reply."
    } else {
        "按当前直播语言使用自然口语化的简体中文，只输出一到两句可直接朗读的正文。不要出现英文、舞台动作、括号说明或表情。严格遵守下方参考中的回复规则，只依据提供的资料和真实直播事件回答事实。明确要求跳过或缺少事实依据时只输出「不用回复」。"
    };
    let mut system = guard.to_owned();
    if !persona.trim().is_empty() {
        system.push_str("\n\n");
        system.push_str(persona);
    }
    if !knowledge.trim().is_empty() {
        system.push_str(if english {
            "\n\n[Reference rules]\n"
        } else {
            "\n\n【角色提示词】\n"
        });
        system.push_str(knowledge);
    }
    system
}

pub fn is_skip_reply(text: &str) -> bool {
    let cleaned = text.trim().trim_matches(|c: char| {
        c.is_whitespace() || "。！？!?.，,：:「」\"'（）()[]【】".contains(c)
    });
    if cleaned.is_empty() {
        return true;
    }
    let lower = cleaned.to_ascii_lowercase();
    if matches!(
        lower.as_str(),
        "不用回复"
            | "不回复"
            | "无需回复"
            | "不需要回复"
            | "不用回"
            | "略过"
            | "no_reply"
            | "skip"
    ) {
        return true;
    }
    cleaned.chars().count() <= 16
        && ["不用回复", "无需回复", "不需要回复", "no_reply", "no reply"]
            .iter()
            .any(|token| lower.contains(token))
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

#[derive(Default, Clone, Serialize)]
pub struct GenerationStats {
    pub first_token_ms: Option<u64>,
    pub first_sentence_ms: Option<u64>,
    pub prompt_tokens: Option<u64>,
    pub cached_tokens: Option<u64>,
}

#[derive(Default)]
struct Stream {
    pending: String,
    total: usize,
    finished: bool,
    truncated: bool,
    stats: GenerationStats,
}

fn trim_truncated_tail(pending: &mut String, already_emitted: bool) {
    let trimmed = pending.trim();
    if trimmed.is_empty() || trimmed.ends_with(['。', '！', '？', '!', '?', '…', '～', '~']) {
        return;
    }
    if already_emitted {
        pending.clear();
        return;
    }
    if let Some((idx, ch)) = trimmed
        .char_indices()
        .rev()
        .find(|(_, c)| "，、,.；;：:".contains(*c))
        && trimmed[..idx].chars().count() >= 6
    {
        let cut = idx + ch.len_utf8();
        let kept = trimmed[..cut].to_owned();
        *pending = kept;
    }
}

impl Stream {
    async fn emit(&mut self, flush: bool, tx: &Sender<String>, start: Instant) -> Result<()> {
        while let Some(line) = split_ready(&mut self.pending, false) {
            if !line.is_empty() {
                self.stats
                    .first_sentence_ms
                    .get_or_insert_with(|| elapsed(start));
                tx.send(line).await.context("本次回复已结束")?;
            }
        }
        if flush {
            if self.truncated {
                trim_truncated_tail(&mut self.pending, self.stats.first_sentence_ms.is_some());
            }
            while let Some(line) = split_ready(&mut self.pending, true) {
                if !line.is_empty() {
                    self.stats
                        .first_sentence_ms
                        .get_or_insert_with(|| elapsed(start));
                    tx.send(line).await.context("本次回复已结束")?;
                }
            }
        }
        Ok(())
    }

    async fn event(
        &mut self,
        line: &[u8],
        tx: &Sender<String>,
        start: Instant,
        usage: bool,
    ) -> Result<bool> {
        let line = std::str::from_utf8(line)?.trim();
        let Some(data) = line.strip_prefix("data:").map(str::trim) else {
            return Ok(false);
        };
        if data == "[DONE]" {
            self.finished = true;
            return Ok(true);
        }
        let value: Value = serde_json::from_str(data).context("语言模型返回了无效的流式数据")?;
        ensure!(value.get("error").is_none(), "语言模型中止了回复");
        if let Some(count) = value
            .pointer("/usage/prompt_tokens")
            .and_then(Value::as_u64)
        {
            self.stats.prompt_tokens = Some(count);
            self.stats.cached_tokens = value
                .pointer("/usage/prompt_tokens_details/cached_tokens")
                .and_then(Value::as_u64);
        }
        if let Some(delta) = value
            .pointer("/choices/0/delta/content")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
        {
            self.stats
                .first_token_ms
                .get_or_insert_with(|| elapsed(start));
            self.total += delta.len();
            ensure!(self.total <= 12_000, "单次模型回复超出长度限制");
            self.pending.push_str(delta);
        }
        if let Some(reason) = value
            .pointer("/choices/0/finish_reason")
            .and_then(Value::as_str)
        {
            self.finished = true;
            if reason == "length" {
                self.truncated = true;
            }
        }
        self.emit(self.finished, tx, start).await?;
        Ok(self.finished && (!usage || self.stats.prompt_tokens.is_some()))
    }
}

pub fn elapsed(start: Instant) -> u64 {
    start.elapsed().as_millis() as u64
}

pub async fn sentences(
    http: &reqwest::Client,
    c: &Settings,
    system: &str,
    input: &str,
    history: &[(String, String)],
    tx: Sender<String>,
) -> Result<GenerationStats> {
    ensure!(
        !c.llm_endpoint.is_empty() && !c.llm_model.trim().is_empty(),
        "请在设置中填写语言模型接口与模型名"
    );
    let mut messages = vec![json!({"role": "system", "content": system})];
    for (prompt, reply) in history {
        messages.push(json!({"role":"user","content":prompt}));
        messages.push(json!({"role":"assistant","content":reply}));
    }
    messages.push(json!({"role":"user","content":input}));
    let usage = c.llm_endpoint == "http://127.0.0.1:19847/v1/chat/completions";
    let mut body = json!({
        "model": c.llm_model,
        "messages": messages,
        "stream": true,
        "max_tokens": 96,
        "temperature": 0.65,
        "top_p": 0.9
    });
    if usage {
        body["stream_options"] = json!({"include_usage": true});
    }
    let mut req = http.post(&c.llm_endpoint).timeout(Duration::from_secs(90));
    if let Some(k) = key("llm")? {
        req = req.bearer_auth(k);
    }
    let start = Instant::now();
    let resp = req.json(&body).send().await.map_err(|e| {
        anyhow::anyhow!(
            "无法连接语言模型（请先在「设置」点击「启动本机对话」）：{}",
            e.without_url()
        )
    })?;
    read_stream(checked(resp, "语言模型").await?, start, tx, usage).await
}

async fn read_stream(
    resp: reqwest::Response,
    start: Instant,
    tx: Sender<String>,
    usage: bool,
) -> Result<GenerationStats> {
    let mut stream = resp.bytes_stream();
    let mut buf = Vec::new();
    let mut response = Stream::default();
    'outer: loop {
        let chunk = if response.finished {
            tokio::time::timeout(Duration::from_millis(200), stream.next())
                .await
                .ok()
                .flatten()
        } else {
            stream.next().await
        };
        let Some(chunk) = chunk else { break };
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(_) if response.finished => break,
            Err(err) => return Err(err.without_url().into()),
        };
        buf.extend_from_slice(&chunk);
        ensure!(buf.len() < 256_000, "模型事件过大");
        while let Some(i) = buf.iter().position(|b| *b == b'\n') {
            let line = buf.drain(..=i).collect::<Vec<_>>();
            if response.event(&line, &tx, start, usage).await? {
                break 'outer;
            }
        }
    }
    if !response.finished && !buf.is_empty() {
        response.event(&buf, &tx, start, usage).await?;
    }
    ensure!(response.finished, "语言模型连接意外中断");
    ensure!(response.total > 0, "语言模型未返回可朗读的文字");
    response.emit(true, &tx, start).await?;
    Ok(response.stats)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streamed_chinese_keeps_every_character() {
        let original = crate::prompts::Profile::defaults(false).persona;
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
    fn skips_no_reply_outputs() {
        assert!(is_skip_reply("不用回复"));
        assert!(is_skip_reply("「不用回复。」"));
        assert!(is_skip_reply("不需要回复"));
        assert!(is_skip_reply("SKIP"));
        assert!(is_skip_reply("好的，不用回复。"));
        assert!(is_skip_reply("这个问题不用回复。"));
        assert!(is_skip_reply("No reply needed."));
        assert!(!is_skip_reply("谢谢老板送的玫瑰"));
        assert!(!is_skip_reply("如果不回复的话你会不会生气呀"));
    }

    #[test]
    fn trims_truncated_trailing_fragment() {
        let mut after_sentence = "如果你喜欢的话可以点个右上角的".to_owned();
        trim_truncated_tail(&mut after_sentence, true);
        assert!(after_sentence.is_empty());

        let mut complete = "谢谢大家来听歌！".to_owned();
        trim_truncated_tail(&mut complete, true);
        assert_eq!(complete, "谢谢大家来听歌！");

        let mut single_clause = "欢迎来到深夜录音房，今晚想听什么歌都可".to_owned();
        trim_truncated_tail(&mut single_clause, false);
        assert_eq!(single_clause, "欢迎来到深夜录音房，");
    }
}
