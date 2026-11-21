use crate::{
    config, dialogue,
    prompts::{self, Answer, Scene, Snapshot},
};
use serde_json::json;
use std::collections::{HashMap, VecDeque};

const HISTORY_TTL_MS: u64 = 5 * 60 * 1000;

pub struct Chat {
    pub text: String,
    pub viewer: Option<String>,
    pub viewer_id: Option<String>,
}

impl From<String> for Chat {
    fn from(text: String) -> Self {
        Self {
            text,
            viewer: None,
            viewer_id: None,
        }
    }
}

impl Chat {
    pub fn model_input(&self, english: bool) -> String {
        match &self.viewer {
            Some(name) if english => format!("{name} says: {}", self.text),
            Some(name) => format!("观众「{name}」说：{}", self.text),
            None => self.text.clone(),
        }
    }

    fn follows_up(&self) -> bool {
        let text = self.text.trim_start().to_ascii_lowercase();
        [
            "刚才",
            "你刚",
            "你说的",
            "那你",
            "那这",
            "那换",
            "为什么",
            "接着",
            "真的吗",
            "还有吗",
            "怎么说",
            "展开说说",
            "不是这个意思",
            "可是",
            "但是",
            "不过",
            "why",
            "you said",
            "what about",
            "and then",
        ]
        .iter()
        .any(|prefix| text.starts_with(prefix))
            || self
                .text
                .trim_end_matches(['?', '？', '。', ' '])
                .ends_with('呢')
    }
}

pub enum Event {
    Ambient,
    Join(String),
    Gift {
        viewer: Option<String>,
        name: String,
        description: String,
        count: u32,
        comment: Option<String>,
    },
}

pub enum Decision {
    Skip,
    Speak(String),
    Generate(String),
}

impl From<Option<String>> for Decision {
    fn from(line: Option<String>) -> Self {
        match line {
            Some(line) if !dialogue::is_skip_reply(&line) => Self::Speak(line),
            _ => Self::Skip,
        }
    }
}

#[derive(Default)]
pub struct Variants {
    recent: HashMap<Option<Scene>, VecDeque<String>>,
}

impl Variants {
    pub fn scene(
        &mut self,
        profile: &Snapshot,
        scene: Scene,
        fields: &[(&str, &str)],
    ) -> Option<String> {
        self.pick(Some(scene), profile.lines(scene))
            .and_then(|line| prompts::render(line, fields))
    }

    fn pick<'a>(&mut self, group: Option<Scene>, lines: &'a [String]) -> Option<&'a str> {
        let recent = self.recent.entry(group).or_default();
        // Prefer unused phrases and fall back to the least recently heard variant.
        let rank = |line: &String| {
            recent
                .iter()
                .position(|played| played == line)
                .map_or(0, |i| i + 1)
        };
        let best = lines.iter().map(rank).min()?;
        let choices: Vec<_> = lines.iter().filter(|line| rank(line) == best).collect();
        let line = choices[uuid::Uuid::new_v4().as_u128() as usize % choices.len()];
        recent.retain(|played| played != line);
        recent.push_back(line.clone());
        if recent.len() > 3 {
            recent.pop_front();
        }
        Some(line)
    }

    pub fn chat(&mut self, profile: &Snapshot, chat: &Chat, english: bool) -> Decision {
        match profile.chat_answer(&chat.text) {
            Some(Answer::Skip) => Decision::Skip,
            Some(Answer::Exact(line)) => Decision::Speak(line.clone()),
            Some(Answer::Variants(lines)) => self.pick(None, lines).map(str::to_owned).into(),
            None => Decision::Generate(chat.model_input(english)),
        }
    }

    pub fn event(&mut self, profile: &Snapshot, event: Event) -> Decision {
        match event {
            Event::Ambient => self.scene(profile, Scene::Ambient, &[]).into(),
            Event::Join(viewer) => self
                .scene(profile, Scene::Join, &[("viewer", &viewer)])
                .into(),
            Event::Gift {
                viewer,
                name,
                description,
                count,
                comment,
            } => {
                if let Some(comment) =
                    comment.filter(|_| !profile.gift_instruction.trim().is_empty())
                {
                    let data = json!({"event":"gift", "viewer":viewer, "gift":name, "count":count, "comment":comment});
                    return Decision::Generate(format!("{}\n{data}", profile.gift_instruction));
                }
                if let Some(exact) = profile.gift_answer(&name, count, &description) {
                    return Some(exact).into();
                }
                match viewer {
                    Some(viewer) => self.scene(
                        profile,
                        Scene::GiftNamed,
                        &[("viewer", &viewer), ("gift", &description)],
                    ),
                    None => self.scene(profile, Scene::GiftAnonymous, &[("gift", &description)]),
                }
                .into()
            }
        }
    }
}

#[derive(Default)]
pub struct History {
    version: String,
    turns: VecDeque<(u64, String, String, String)>,
}

impl History {
    pub fn for_chat(&self, chat: &Chat, version: &str) -> Vec<(String, String)> {
        self.for_chat_at(chat, version, config::now())
    }

    fn for_chat_at(&self, chat: &Chat, version: &str, now_ms: u64) -> Vec<(String, String)> {
        if self.version != version {
            return Vec::new();
        }
        let Some(id) = chat.viewer_id.as_ref().filter(|_| chat.follows_up()) else {
            return Vec::new();
        };
        let mut turns: Vec<_> = self
            .turns
            .iter()
            .rev()
            .filter(|(at, viewer, _, _)| {
                viewer == id && now_ms.saturating_sub(*at) <= HISTORY_TTL_MS
            })
            .take(2)
            .map(|(_, _, input, reply)| (input.clone(), reply.clone()))
            .collect();
        turns.reverse();
        turns
    }

    pub fn remember(&mut self, chat: &Chat, version: &str, input: String, answer: String) {
        self.remember_at(chat, version, input, answer, config::now());
    }

    fn remember_at(
        &mut self,
        chat: &Chat,
        version: &str,
        input: String,
        answer: String,
        now_ms: u64,
    ) {
        if let Some(id) = &chat.viewer_id {
            if self.version != version {
                self.turns.clear();
                self.version = version.into();
            }
            while self
                .turns
                .front()
                .is_some_and(|(at, _, _, _)| now_ms.saturating_sub(*at) > HISTORY_TTL_MS)
            {
                self.turns.pop_front();
            }
            if self.turns.len() >= 64 {
                self.turns.pop_front();
            }
            self.turns.push_back((now_ms, id.clone(), input, answer));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prompts::Profile;

    #[test]
    fn gift_with_comment_goes_to_model_instead_of_static_rule() {
        let knowledge = "\
Q：观众送了小心心、点赞、玫瑰、灯牌等小礼物怎么感谢？
A：谢谢宝子送的什么什么礼物，谢谢老板

Q：观众送了跑车、火箭、嘉年华、城堡等贵重礼物或连续刷礼物怎么感谢？
A：感谢老板的礼物，老板大气
";
        let mut profile = Profile::defaults(false);
        profile.retain_resolved_defaults(knowledge);
        let snap = profile.compile(knowledge, false, "v1".into()).unwrap();
        let mut variants = Variants::default();

        let with_comment = variants.event(
            &snap,
            Event::Gift {
                viewer: Some("小明".into()),
                name: "玫瑰".into(),
                description: "玫瑰".into(),
                count: 1,
                comment: Some("唱首桥边姑娘吧".into()),
            },
        );
        assert!(
            matches!(with_comment, Decision::Generate(ref text) if text.contains("唱首桥边姑娘吧"))
        );

        let without_comment = variants.event(
            &snap,
            Event::Gift {
                viewer: Some("小明".into()),
                name: "玫瑰".into(),
                description: "玫瑰".into(),
                count: 1,
                comment: None,
            },
        );
        assert!(
            matches!(without_comment, Decision::Speak(ref line) if line == "谢谢宝子送的玫瑰，谢谢老板")
        );
    }

    #[test]
    fn history_respects_ttl_and_follow_up_cues() {
        let mut history = History::default();
        let first = Chat {
            text: "今晚唱什么歌".into(),
            viewer: Some("小明".into()),
            viewer_id: Some("yt/v1".into()),
        };
        history.remember_at(
            &first,
            "v1",
            "观众「小明」说：今晚唱什么歌".into(),
            "唱桥边姑娘".into(),
            1_000,
        );

        let follow = Chat {
            text: "真的吗，那换一首呢".into(),
            viewer: Some("小明".into()),
            viewer_id: Some("yt/v1".into()),
        };
        assert_eq!(history.for_chat_at(&follow, "v1", 1_000 + 60_000).len(), 1);
        assert!(
            history
                .for_chat_at(&follow, "v1", 1_000 + HISTORY_TTL_MS + 1)
                .is_empty()
        );
        assert!(
            history
                .for_chat_at(&follow, "v2", 1_000 + 60_000)
                .is_empty()
        );
    }
}
