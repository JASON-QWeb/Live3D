use crate::dialogue;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Scene {
    Ambient,
    Join,
    GiftNamed,
    GiftAnonymous,
    SongIntro,
    SongEnd,
    SongGifts,
}

impl Scene {
    fn fields(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Self::Ambient => &[],
            Self::Join => &[("viewer", "")],
            Self::GiftNamed => &[("viewer", ""), ("gift", "")],
            Self::GiftAnonymous => &[("gift", "")],
            Self::SongIntro | Self::SongEnd => &[("song", "")],
            Self::SongGifts => &[("song", ""), ("gifts", "")],
        }
    }
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "source", rename_all = "snake_case", deny_unknown_fields)]
pub enum Knowledge {
    Inline { text: String },
    AvatarFile,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub persona: String,
    pub knowledge: Knowledge,
    pub events: BTreeMap<Scene, Vec<String>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    chat_rules: Vec<ChatRule>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    gift_rules: Vec<GiftRule>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    ambient_qa_prefix: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    gift_instruction: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ChatRule {
    matches: Vec<String>,
    reply: RuleReply,
}

#[derive(Deserialize, Serialize)]
#[serde(untagged, deny_unknown_fields)]
enum RuleReply {
    Qa { qa: String },
    Say { lines: Vec<String> },
    Skip { skip: bool },
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct GiftRule {
    names: Vec<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    multiple: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    min_count: Option<u32>,
    qa: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    gift_slot: String,
}

#[derive(Clone)]
pub enum Answer {
    Skip,
    Exact(String),
    Variants(Vec<String>),
}

pub struct Snapshot {
    pub version: String,
    pub system: String,
    pub gift_instruction: String,
    events: BTreeMap<Scene, Vec<String>>,
    chat: HashMap<String, Answer>,
    gifts: Vec<(GiftRule, String)>,
}

fn normalize(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_whitespace() && !"，。！？、；：,.!?;:…～~".contains(*c))
        .flat_map(char::to_lowercase)
        .collect()
}

fn qa_pairs(knowledge: &str) -> Vec<(&str, &str)> {
    let mut pairs = Vec::new();
    let mut question = None;
    for line in knowledge.lines().map(str::trim) {
        if let Some(q) = after_label(line, &["Q：", "Q:", "q：", "q:", "问：", "问:"]) {
            question = Some(q);
        } else if let Some(a) = after_label(line, &["A：", "A:", "a：", "a:", "答：", "答:"])
            && let Some(q) = question.take()
            && !a.is_empty()
        {
            pairs.push((q, a));
        }
    }
    pairs
}

fn after_label<'a>(line: &'a str, labels: &[&str]) -> Option<&'a str> {
    labels
        .iter()
        .find_map(|label| line.strip_prefix(label))
        .map(str::trim)
}

fn referenced_answer<'a>(pairs: &[(&str, &'a str)], question: &str) -> Option<&'a str> {
    pairs
        .iter()
        .find(|(q, _)| q.starts_with(question))
        .map(|(_, answer)| *answer)
}

fn exact_answer(answer: &str) -> Answer {
    let short = answer.trim_end_matches('。').strip_suffix("即可");
    let line = short
        .and_then(|text| {
            ["都回复", "只回复", "仅回复"]
                .iter()
                .find_map(|prefix| text.split_once(prefix).map(|(_, line)| line))
        })
        .map(|line| line.trim().trim_matches(['「', '」', '“', '”', '"']))
        .filter(|line| !line.is_empty())
        .unwrap_or(answer);
    if dialogue::is_skip_reply(line) {
        Answer::Skip
    } else {
        Answer::Exact(line.to_owned())
    }
}

impl Profile {
    pub fn defaults(english: bool) -> Self {
        let json = if english {
            include_str!("../prompts/defaults/en.json")
        } else {
            include_str!("../prompts/defaults/zh.json")
        };
        serde_json::from_str(json).expect("valid bundled speech profile")
    }

    pub fn retain_resolved_defaults(&mut self, knowledge: &str) {
        let pairs = qa_pairs(knowledge);
        self.chat_rules.retain(|rule| match &rule.reply {
            RuleReply::Qa { qa } => referenced_answer(&pairs, qa).is_some(),
            _ => true,
        });
        self.gift_rules
            .retain(|rule| referenced_answer(&pairs, &rule.qa).is_some());
    }

    pub fn compile(mut self, knowledge: &str, english: bool, version: String) -> Result<Snapshot> {
        ensure!(
            self.persona.len() <= 16_000 && knowledge.len() <= 32_000,
            "话术档案文本过长"
        );
        ensure!(self.gift_instruction.len() <= 2000, "礼物留言回复指令过长");
        for (&scene, lines) in &self.events {
            validate_lines(lines, scene.fields()).with_context(|| format!("话术场景 {scene:?}"))?;
        }
        if let Some(lines) = self.events.get_mut(&Scene::Ambient)
            && !lines.is_empty()
            && !self.ambient_qa_prefix.is_empty()
            && let Some(line) = knowledge
                .lines()
                .find_map(|line| line.trim().strip_prefix(&self.ambient_qa_prefix))
                .map(str::trim)
                .filter(|line| !line.is_empty())
            && !lines.iter().any(|entry| entry == line)
        {
            lines.push(line.into());
        }
        let pairs = qa_pairs(knowledge);
        let chat = compile_chat(self.chat_rules, &pairs)?;
        let mut gifts = Vec::new();
        for rule in self.gift_rules {
            ensure!(
                !rule.qa.trim().is_empty()
                    && (rule.multiple || rule.min_count.is_some() || !rule.names.is_empty()),
                "礼物规则需要 QA 问题和匹配条件"
            );
            if let Some(min) = rule.min_count {
                ensure!(min >= 2, "礼物数量阈值须至少为 2");
            }
            let answer = referenced_answer(&pairs, &rule.qa)
                .with_context(|| format!("找不到礼物规则绑定的 QA 问题「{}」", rule.qa))?
                .to_owned();
            gifts.push((rule, answer));
        }
        Ok(Snapshot {
            version,
            system: dialogue::system_prompt(&self.persona, knowledge, english),
            gift_instruction: self.gift_instruction,
            events: self.events,
            chat,
            gifts,
        })
    }
}

fn compile_chat(rules: Vec<ChatRule>, pairs: &[(&str, &str)]) -> Result<HashMap<String, Answer>> {
    let mut result = HashMap::new();
    for (question, answer) in pairs {
        result
            .entry(normalize(question))
            .or_insert_with(|| exact_answer(answer));
    }
    let mut configured = HashSet::new();
    for rule in rules {
        ensure!(!rule.matches.is_empty(), "弹幕规则缺少匹配内容");
        let answer = match rule.reply {
            RuleReply::Qa { qa } => {
                ensure!(!qa.trim().is_empty(), "弹幕规则缺少 QA 问题");
                let answer = referenced_answer(pairs, &qa)
                    .with_context(|| format!("找不到弹幕规则绑定的 QA 问题「{qa}」"))?;
                exact_answer(answer)
            }
            RuleReply::Say { lines } => {
                ensure!(!lines.is_empty(), "弹幕规则缺少话术");
                validate_lines(&lines, &[])?;
                Answer::Variants(lines)
            }
            RuleReply::Skip { skip } => {
                ensure!(skip, "跳过规则须将 skip 设为 true");
                Answer::Skip
            }
        };
        for item in rule.matches {
            let key = normalize(&item);
            ensure!(
                !key.is_empty() && configured.insert(key.clone()),
                "无效或重复的弹幕规则：{item}"
            );
            result.insert(key, answer.clone());
        }
    }
    Ok(result)
}

impl Snapshot {
    pub fn lines(&self, scene: Scene) -> &[String] {
        self.events.get(&scene).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn chat_answer(&self, text: &str) -> Option<&Answer> {
        self.chat.get(&normalize(text))
    }

    pub fn gift_answer(&self, name: &str, count: u32, description: &str) -> Option<String> {
        let name = normalize(name);
        let matched = self
            .gifts
            .iter()
            .find(|(rule, _)| {
                rule.min_count.is_some_and(|min| count >= min)
                    || rule.names.iter().any(|value| normalize(value) == name)
            })
            .or_else(|| {
                self.gifts
                    .iter()
                    .find(|(rule, _)| rule.multiple && rule.min_count.is_none() && count > 1)
            });
        let (rule, line) = matched?;
        Some(if rule.gift_slot.is_empty() {
            line.clone()
        } else {
            line.replace(&rule.gift_slot, description)
        })
    }
}

fn validate_lines(lines: &[String], fields: &[(&str, &str)]) -> Result<()> {
    ensure!(lines.len() <= 50, "每组话术最多 50 条");
    for line in lines {
        ensure!(
            !line.trim().is_empty()
                && line.chars().count() <= 200
                && render(line, fields).is_some(),
            "话术须在 1 至 200 字内，且仅使用该场景的占位符：{line}"
        );
    }
    Ok(())
}

pub fn render(template: &str, values: &[(&str, &str)]) -> Option<String> {
    let mut result = String::with_capacity(template.len());
    let mut rest = template;
    while let Some((prefix, slot)) = rest.split_once('{') {
        if prefix.contains('}') {
            return None;
        }
        let (field, tail) = slot.split_once('}')?;
        let (_, value) = values.iter().find(|(key, _)| *key == field)?;
        result.push_str(prefix);
        result.push_str(value);
        rest = tail;
    }
    if rest.contains('}') {
        return None;
    }
    result.push_str(rest);
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gift_rules_do_not_hijack_small_gift_combos_and_support_min_count() {
        let knowledge = "\
Q：观众送了小心心、点赞、玫瑰、灯牌等小礼物怎么感谢？
A：谢谢宝子送的什么什么礼物，谢谢老板

Q：观众送了跑车、火箭、嘉年华、城堡等贵重礼物或连续刷礼物怎么感谢？
A：感谢老板的礼物，老板大气
";
        let mut profile = Profile::defaults(false);
        profile.retain_resolved_defaults(knowledge);
        let snap = profile.compile(knowledge, false, "v1".into()).unwrap();

        assert_eq!(
            snap.gift_answer("小心心", 2, "2个小心心").as_deref(),
            Some("谢谢宝子送的2个小心心，谢谢老板")
        );
        assert_eq!(
            snap.gift_answer("跑车", 1, "跑车").as_deref(),
            Some("感谢老板的礼物，老板大气")
        );
        assert_eq!(
            snap.gift_answer("玫瑰", 10, "10个玫瑰").as_deref(),
            Some("感谢老板的礼物，老板大气")
        );
    }

    #[test]
    fn legacy_multiple_gift_rule_prefers_explicit_name_match() {
        let knowledge = "\
Q：观众送了小心心、点赞、玫瑰、灯牌等小礼物怎么感谢？
A：谢谢宝子送的什么什么礼物，谢谢老板

Q：观众送了跑车、火箭、嘉年华、城堡等贵重礼物或连续刷礼物怎么感谢？
A：感谢老板的礼物，老板大气
";
        let mut profile = Profile::defaults(false);
        profile.gift_rules[0].min_count = None;
        profile.gift_rules[0].multiple = true;
        profile.retain_resolved_defaults(knowledge);
        let snap = profile.compile(knowledge, false, "v1".into()).unwrap();

        assert_eq!(
            snap.gift_answer("小心心", 2, "2个小心心").as_deref(),
            Some("谢谢宝子送的2个小心心，谢谢老板")
        );
        assert_eq!(
            snap.gift_answer("神秘礼物", 3, "3个神秘礼物").as_deref(),
            Some("感谢老板的礼物，老板大气")
        );
    }

    #[test]
    fn chat_rules_override_auto_indexed_qa_pairs() {
        let knowledge = "Q：你好\nA：原答案\n";
        let profile = Profile {
            persona: "测试".into(),
            knowledge: Knowledge::Inline {
                text: knowledge.into(),
            },
            events: BTreeMap::new(),
            chat_rules: vec![ChatRule {
                matches: vec!["你好".into()],
                reply: RuleReply::Skip { skip: true },
            }],
            gift_rules: Vec::new(),
            ambient_qa_prefix: String::new(),
            gift_instruction: String::new(),
        };
        let snap = profile.compile(knowledge, false, "v1".into()).unwrap();
        assert!(matches!(snap.chat_answer("你好"), Some(Answer::Skip)));
    }
}
