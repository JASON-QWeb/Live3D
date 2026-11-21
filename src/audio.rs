use crate::config::Settings;
use anyhow::{Context, Result, anyhow, ensure};
use bytes::Bytes;
use futures_util::StreamExt;
use serde_json::json;
use std::{path::PathBuf, process::Stdio, time::Duration};
use tokio::{io::AsyncWriteExt, process::Command};

const MAX_AUDIO: usize = 32 * 1024 * 1024;
pub const FRAME: usize = 960; // 20 ms at 24 kHz, signed 16-bit mono
pub fn ffmpeg() -> Option<PathBuf> {
    [
        "/opt/homebrew/bin/ffmpeg",
        "/usr/local/bin/ffmpeg",
        "/opt/local/bin/ffmpeg",
    ]
    .into_iter()
    .map(PathBuf::from)
    .chain(
        std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
            .map(|p| p.join("ffmpeg")),
    )
    .find(|p| p.is_file())
}
pub async fn limited(resp: reqwest::Response, cap: usize) -> Result<Vec<u8>> {
    ensure!(
        resp.content_length().unwrap_or(0) <= cap as u64,
        "服务返回的数据过大"
    );
    let mut data = Vec::new();
    let mut stream = resp.bytes_stream();
    while let Some(piece) = stream.next().await {
        let b = piece.map_err(|e| e.without_url())?;
        ensure!(data.len() + b.len() <= cap, "服务返回的数据过大");
        data.extend_from_slice(&b);
    }
    Ok(data)
}
pub async fn checked(r: reqwest::Response, service: &str) -> Result<reqwest::Response> {
    let status = r.status();
    if status.is_success() {
        return Ok(r);
    }
    // Only publish machine codes. Third-party error messages may echo credentials or signed URLs.
    let v =
        serde_json::from_slice::<serde_json::Value>(&limited(r, 32_768).await.unwrap_or_default())
            .unwrap_or_default();
    let code = v
        .get("code")
        .or_else(|| v.pointer("/error/status"))
        .and_then(|v| v.as_str())
        .filter(|s| {
            s.len() < 80
                && s.bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"_-.".contains(&c))
        })
        .unwrap_or("");
    Err(anyhow!("{service} 返回 HTTP {} {code}", status.as_u16()))
}
pub async fn decode(data: &[u8]) -> Result<Bytes> {
    ensure!(
        data.len() <= MAX_AUDIO && !data.is_empty(),
        "音频为空或超过 32 MiB"
    );
    let exe = ffmpeg().context("请安装 ffmpeg（brew install ffmpeg）")?;
    let mut c = Command::new(exe)
        .kill_on_drop(true)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostdin",
            "-protocol_whitelist",
            "pipe",
            "-i",
            "pipe:0",
            "-map",
            "0:a:0",
            "-vn",
            "-t",
            "601",
            "-ar",
            "24000",
            "-ac",
            "1",
            "-f",
            "s16le",
            "pipe:1",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let mut input = c.stdin.take().context("无法打开音频管道")?;
    let write = async {
        let res = input.write_all(data).await;
        drop(input);
        res
    };
    let op = async {
        let (_, out) = tokio::join!(write, c.wait_with_output());
        out
    };
    let out = tokio::time::timeout(Duration::from_secs(30), op).await??;
    ensure!(
        out.status.success() && !out.stdout.is_empty(),
        "无法解码音频，请使用包含音轨的文件"
    );
    ensure!(out.stdout.len() <= 600 * 48_000, "单条音频最长 10 分钟");
    Ok(out.stdout.into())
}
async fn mac_voice(text: &str) -> Result<Vec<u8>> {
    let tmp = tempfile::Builder::new().suffix(".aiff").tempfile()?;
    let mut cmd = Command::new("/usr/bin/say");
    cmd.kill_on_drop(true)
        .args(["-v", "Tingting", "-o"])
        .arg(tmp.path())
        .arg("--")
        .arg(text)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let status = tokio::time::timeout(Duration::from_secs(65), cmd.status()).await??;
    ensure!(
        status.success(),
        "系统语音生成失败，请检查 macOS 中文语音安装"
    );
    Ok(tokio::fs::read(tmp.path()).await?)
}
pub async fn synth(http: &reqwest::Client, c: &Settings, text: &str) -> Result<Bytes> {
    let audio = if c.tts_mode == "system" {
        mac_voice(text).await?
    } else {
        let mut body = json!({"model":c.tts_model,"input":text,"response_format":"wav","lang_code":"z",
            "language":"chinese"});
        if !c.voice.trim().is_empty() {
            body["voice"] = json!(c.voice);
        }
        if !c.reference_audio.is_empty() {
            body["ref_audio"] = json!(c.reference_audio);
            body["ref_text"] = json!(c.reference_text);
        }
        let resp = http
            .post(&c.tts_endpoint)
            .timeout(Duration::from_secs(120))
            .json(&body)
            .send()
            .await
            .map_err(|e| anyhow!("无法连接语音服务：{}", e.without_url()))?;
        limited(checked(resp, "语音服务").await?, MAX_AUDIO).await?
    };
    decode(&audio).await
}
pub fn wav(pcm: &[u8]) -> Vec<u8> {
    let len = pcm.len() as u32;
    let mut out = Vec::with_capacity(44 + pcm.len());
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    for v in [16u32, 0x00010001, 24000, 48000, 0x00100002] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(b"data");
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(pcm);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_clock_agrees_with_wire_frame_duration() {
        let header = wav(&[]);
        let bytes_per_second = u32::from_le_bytes(header[28..32].try_into().unwrap());
        assert_eq!(u16::from_le_bytes(header[22..24].try_into().unwrap()), 1);
        assert_eq!(
            u32::from_le_bytes(header[24..28].try_into().unwrap()),
            24_000
        );
        assert_eq!(u16::from_le_bytes(header[34..36].try_into().unwrap()), 16);
        assert_eq!(FRAME as u32, bytes_per_second * 20 / 1000);
        assert_eq!(&header[36..40], b"data");
        assert_eq!(header.len(), 44);
    }
}
