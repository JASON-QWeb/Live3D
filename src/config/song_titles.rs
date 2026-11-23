use super::Settings;
use crate::paths::song_titles_file_path;
use std::fs;

fn strip_audio_ext(s: &str) -> &str {
    let trimmed = s.trim().trim_start_matches('\u{feff}').trim();
    const EXTS: [&str; 9] = [
        ".mp3", ".wav", ".flac", ".m4a", ".aac", ".ogg", ".opus", ".mp4", ".mov",
    ];
    let lower = trimmed.to_ascii_lowercase();
    for ext in EXTS {
        if lower.ends_with(ext) && trimmed.len() > ext.len() {
            return trimmed[..trimmed.len() - ext.len()].trim();
        }
    }
    trimmed
}

fn strip_cover_suffix(s: &str) -> &str {
    let mut cur = s.trim();
    const SUFFIXES: [&str; 7] = [
        "－小柔演唱",
        "-小柔演唱",
        "—小柔演唱",
        "_小柔演唱",
        "（小柔演唱）",
        "(小柔演唱)",
        "小柔演唱",
    ];
    loop {
        let mut changed = false;
        for suf in SUFFIXES {
            if let Some(rest) = cur.strip_suffix(suf) {
                let next = rest
                    .trim_end_matches(|c: char| {
                        c.is_whitespace() || matches!(c, '-' | '－' | '—' | '–' | '_' | '·')
                    })
                    .trim();
                if !next.is_empty() {
                    cur = next;
                    changed = true;
                    break;
                }
            }
        }
        if !changed {
            break;
        }
    }
    cur
}

fn normalize_song_key(raw: &str) -> String {
    let base = strip_cover_suffix(strip_audio_ext(raw));
    let mut out = String::with_capacity(base.len());
    let mut prev_space = false;
    for ch in base.chars() {
        if ch.is_whitespace() {
            if !prev_space && !out.is_empty() {
                out.push(' ');
                prev_space = true;
            }
        } else {
            out.push(ch.to_ascii_lowercase());
            prev_space = false;
        }
    }
    out.trim().to_owned()
}

fn split_song_mapping_line(line: &str) -> Option<(&str, &str)> {
    let trimmed = line.trim().trim_start_matches('\u{feff}').trim();
    if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with("//") {
        return None;
    }
    for sep in ["=>", "->", "=", "＝", "\t"] {
        if let Some((k, v)) = trimmed.split_once(sep) {
            let key = k.trim();
            let val = v.trim();
            if !key.is_empty() && !val.is_empty() {
                return Some((key, val));
            }
        }
    }
    None
}

fn strip_leading_track_number(s: &str) -> &str {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i == 0 || i == bytes.len() {
        return s;
    }
    let rest = &s[i..];
    if let Some(after_delim) = rest.strip_prefix(['.', '、', '_', '-', '－', ' ', '\t']) {
        let cleaned = after_delim.trim_start_matches(['.', '、', '_', '-', '－', ' ', '\t']);
        if !cleaned.is_empty() {
            return cleaned;
        }
    }
    // Handle compact prefixes like "97云烟成雨" or "101红豆" while preserving titles like "2002年的第一场雪".
    if let Some(first) = rest.chars().next()
        && !first.is_ascii()
        && !matches!(
            first,
            '年' | '月' | '日' | '度' | '分' | '秒' | '号' | '岁' | '夜' | '天'
        )
    {
        return rest.trim();
    }
    s
}

fn strip_bracketed_notes(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut depth_paren = 0u32;
    let mut depth_bracket = 0u32;
    for ch in s.chars() {
        match ch {
            '(' | '（' => depth_paren += 1,
            ')' | '）' if depth_paren > 0 => depth_paren -= 1,
            '[' | '【' => depth_bracket += 1,
            ']' | '】' if depth_bracket > 0 => depth_bracket -= 1,
            _ if depth_paren == 0 && depth_bracket == 0 => out.push(ch),
            _ => {}
        }
    }
    out.trim().to_owned()
}

pub fn clean_fallback_song_title(raw: &str) -> String {
    let stem = strip_audio_ext(raw);
    let base = strip_cover_suffix(stem);
    let no_num = strip_leading_track_number(base);
    let first_part = no_num
        .split(['-', '－', '—', '–'])
        .map(str::trim)
        .find(|part| !part.is_empty())
        .unwrap_or(no_num);
    let cleaned = strip_bracketed_notes(first_part);
    if cleaned.is_empty() {
        stem.to_owned()
    } else {
        cleaned
    }
}

pub fn resolve_song_title_from_text(raw: &str, mapping_text: Option<&str>) -> String {
    let stem = strip_audio_ext(raw);
    if stem.is_empty() {
        return raw.trim().to_owned();
    }
    if let Some(text) = mapping_text {
        let query_norm = normalize_song_key(stem);
        let mut normalized_match: Option<&str> = None;
        for line in text.lines() {
            if let Some((k, v)) = split_song_mapping_line(line) {
                if strip_audio_ext(k) == stem {
                    return v.to_owned();
                }
                if normalized_match.is_none() && normalize_song_key(k) == query_norm {
                    normalized_match = Some(v);
                }
            }
        }
        if let Some(v) = normalized_match {
            return v.to_owned();
        }
    }
    clean_fallback_song_title(stem)
}

pub fn resolve_song_title(cfg: &Settings, raw: &str) -> String {
    let mapping = song_titles_file_path(cfg.is_english())
        .and_then(|p| fs::read_to_string(p).ok())
        .filter(|s| s.len() <= 256_000);
    resolve_song_title_from_text(raw, mapping.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resolves_song_titles_from_mapping_and_fallback() {
        let map = "\
# comment
223. 海伦-桥边姑娘－小柔演唱 = 桥边姑娘
97云烟成雨-《我是江小白》动画片尾曲 - 房东的猫－小柔演唱 = 云烟成雨
安和桥-宋冬野 = 安和桥
";
        assert_eq!(
            resolve_song_title_from_text("223. 海伦-桥边姑娘－小柔演唱", Some(map)),
            "桥边姑娘"
        );
        // Works with or without the "－小柔演唱" suffix and file extension
        assert_eq!(
            resolve_song_title_from_text("安和桥-宋冬野－小柔演唱.mp3", Some(map)),
            "安和桥"
        );
        assert_eq!(
            resolve_song_title_from_text("223. 海伦-桥边姑娘", Some(map)),
            "桥边姑娘"
        );
        // Full-width equals sign from Chinese IME also works
        assert_eq!(
            resolve_song_title_from_text(
                "陈奕迅-好久不见－小柔演唱",
                Some("陈奕迅-好久不见 ＝ 好久不见")
            ),
            "好久不见"
        );
        // Fallback cleans up unmapped filenames cleanly while keeping numeric song titles
        assert_eq!(
            resolve_song_title_from_text(
                "97云烟成雨-《我是江小白》动画片尾曲 - 房东的猫－小柔演唱",
                None
            ),
            "云烟成雨"
        );
        assert_eq!(
            resolve_song_title_from_text("090.旅行的意义(国) - 陈绮贞－小柔演唱", None),
            "旅行的意义"
        );
        assert_eq!(
            resolve_song_title_from_text("2002年的第一场雪 - 刀郎", None),
            "2002年的第一场雪"
        );
    }
}
