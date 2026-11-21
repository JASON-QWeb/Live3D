# Avatar3D Live

Avatar3D Live 是一个在 Apple Silicon Mac 上运行的数字人直播控制台。你可以让角色念台词、回弹幕、做动作，再用 OBS 推流。对话和语音合成可以在 Mac 上跑，角色画面由 Vidu S2 根据一张单人图或你账户里的形象 ID 实时生成。目前还不支持加载 VRM、GLB 模型或做骨骼渲染。

[素材来源与授权](NOTICE.md) · [MIT 许可证](LICENSE)

## 在 Mac 上打开

双击「Avatar3D Live.app」，控制台会在默认浏览器里打开。关掉网页后可以双击应用再打开。要关掉只在本机运行的后台服务，到「设置」里点「退出 Avatar3D」。

应用在 Apple M4 上构建和测试，使用本地签名。新机器需要 FFmpeg，跑本机模型还需要 uv。用 Homebrew 安装可以运行 `brew install ffmpeg uv`。推流需要 OBS Studio 28 或更新版本。[下载 OBS](https://obsproject.com/download)

在「设置」里点「启动本机 MLX」和「启动本机对话」。第一次启动会安装 Python 环境并下载模型，等两项都显示「已就绪」就可以试听。对话默认用 Qwen2.5-1.5B-Instruct 4bit，也可以填兼容的服务地址。若要用参考录音，语音服务得支持这个功能，还要填录音的逐字稿。[MLX Audio](https://github.com/Blaizzy/mlx-audio) · [MLX LM](https://github.com/ml-explore/mlx-lm)

## 先排练，再连线

1. 在「直播台 → 直接说话」输入一句台词，点「本机试听」。切到「AI 回复」再试一次，就能听到从生成回答到念出来的效果。这两步都可以在 Vidu 待机时完成。
2. 打开 OBS，在「设置 → 视频」选好画布和输出尺寸。竖屏可以用 720 × 1280。到「工具 → WebSocket 服务器设置」启用服务器。回到 Avatar3D Live，在「设置 → OBS 场景」填上本机端口和密码，点「建立 Avatar3D 场景」，再到 OBS 选中「Avatar3D · 数字人」。
3. 确认舞台显示 WebRTC 和 H.264 可用。在 OBS 开始本地录制，回到控制台点「舞台试音」，然后看 OBS 混音器有没有电平，回放里能不能听到声音。也可以点「复制 OBS 地址」，把地址手动加到 OBS 的浏览器源，勾选通过 OBS 控制音频。浏览器里的独立舞台和 OBS 共用一个 RTC 观看席位，切到 OBS 前先关掉独立舞台。
4. 在「设置」里保存 Vidu 密钥，上传单人角色图或填写账户里的形象 ID。图片支持 PNG、JPEG 和 WebP，最大 8 MiB。两项都填了就用形象 ID。选好分辨率、动作库和连线时限后点「保存设置」。默认时限是 5 分钟。
5. 点「开始连线」，确认时限和估算费用。连接就绪后，让角色说句话、做个动作，看看画面什么时候出来，人物有没有走样，嘴型能不能和声音对上。也回看一遍 OBS 录像。正式推流的开始和停止都在 OBS 里操作。下播后回 Avatar3D Live 点「结束连线」，等页面显示「Vidu 已确认会话结束」。

Vidu 组件版每秒收 1 积分，折合每分钟 1.875 元。连线 30 分钟约用 1,800 积分。到时限后，应用会向 Vidu 请求结束会话，费用以平台结算为准。断线时，应用只尝试恢复原来的会话。如果没查到会话是否创建或结束，控制台会留一条待核对记录，重启后也能继续查。[Vidu S2 参数与计费](https://platform.vidu.cn/vidu-stream/doc/s2-avatar/component/parameters)

## 弹幕、自动播报与曲库

在「设置 → 语音与直播环境」里选中文或英文。本机 MLX 一次只加载一种语音模型。中文用 Ming 3.0 0.5B 4-bit 的灵小柔，以 MLX 编译和五步推理运行。英文用 ZONOS3-Mini 的 Willow。

进房欢迎、礼物答谢和唱完歌后的谢幕各有自己的话术。弹幕如果命中了指定的问答或跳过规则，就按规则处理。需要根据弹幕或礼物留言现场接话时，由语言模型生成回复。播报一条一条来，两条之间至少隔开设定的时间。

在「设置 → 编辑话术」可以改当前直播语言的话术。文件保存在应用数据目录的 `prompts/<档案名>/` 下，中文是 `zh.json`，英文是 `en.json`。第一次加载时会沿用已保存的人设和欢迎词。如果项目里有 `avatars/cn/xiaorou_knowledge_base.txt`，中文档案会直接用它，改文件后自动生效。设置页会显示当前版本，保存时会检查磁盘上的版本有没有变。

- `events` 按场景填写话术，每个场景可以放多句。应用会尽量避开最近念过的句子。
- `chat_rules` 按弹幕内容匹配，可以用已有的问答、跳过这条弹幕，或准备几条可选回复。
- `gift_rules` 可以按真实礼物名称和数量匹配已有的问答。

只有能可靠认出是同一位观众的追问，才会带上最近两轮对话。没在唱歌时，角色会隔一段时间换句欢迎词，也会偶尔欢迎刚进房的观众。定时唱歌和弹幕点歌期间会暂停欢迎和循环动作。期间收到的礼物先记下来，等整首歌唱完、角色恢复自然神态后再统一答谢。中文直播时，进房欢迎会跳过用户名带英文字母的观众。需要答谢这类观众时，会称呼「老板」或「朋友」。

YouTube 弹幕走 YouTube Data API v3。在「设置」里保存 API Key 和正在直播的视频 ID，再选仅显示、自动朗读或自动回复。抖音、TikTok 等平台可以用有授权的本机采集工具。在设置里点「复制本机弹幕接入信息」，按复制的地址和令牌发送 JSON。默认地址是 `POST http://127.0.0.1:19848/api/ingest`，请求头是 `Authorization: Bearer <接入令牌>`。

| 字段 | 内容 |
| --- | --- |
| `id` | 平台给的稳定事件 ID，用来过滤重复推送 |
| `platform` | 可省略。可填 `douyin`、`tiktok`、`youtube` 或 `local` |
| `kind` | `chat` 是弹幕，`gift` 是礼物，`join` 是进房。省略时按 `gift` 字段处理 |
| `author` | 观众在平台上的显示名 |
| `viewer_id` | 可省略。平台给的稳定观众 ID，用来认出同一个人的追问 |
| `text` | 弹幕原文或礼物留言。进房事件可以留空，礼物事件填了 `gift_name` 也可以留空 |
| `gift` | 兼容旧格式，填 `true` 按礼物处理 |
| `gift_name` | 礼物在平台上的名称 |
| `gift_count` | 礼物数量，默认是 `1` |

在「音频、参考图与曲库」可以播音频、给角色换道具或背景、传服装参考图，也可以让角色回到默认姿态。想让角色轮流做几个动作，在设置里每行写一个，连线后开启循环。

曲库里的音频要手动导入，并有播放授权。每首最大 32 MiB，最长 10 分钟。开启点播后，观众发「点歌」加完整曲名，就能把这首歌排进队列。开启「定时自动唱歌」后，应用会按设置的间隔轮播曲库。

## 实测记录

把录像文件路径传给 `python3 scripts/inspect-recording.py`，可以看解码帧数、平均帧率、最大的帧间隔，以及各条音轨比画面早还是晚。音轨偏移为正值表示声音较晚。时间戳可以帮你找到画面卡顿的位置，检查口型还得回看录像。应用包的 `Contents/Resources/inspect-recording.py` 也有这个脚本。

在 Apple M4、macOS 26.6.2 和 OBS 32.0.4 上，测过本机中文问答和语音、控制台播放、OBS 建场和录制，也确认了 OBS 内置浏览器能用 WebRTC 接收并解码 H.264。另用一套独立的 OBS 配置向本机 RTMP 接收端连续推流 1,805.333 秒。接收端解出 54,160 帧，画面是 720 × 1280，平均 30 fps。测试画面用的是项目图标，接收文件里能找到全部 7 段中文试音。

Vidu S2 做过一次约 5 分钟的非公开联测，用的是一张公版单人照片。等了 33.6 秒出首帧，OBS 录下 5 分 3 秒，Vidu 确认计费 264 秒。录像里能看到人物开口、转头、挥手和双手持杯。打断长句后约 2.25 秒能听到新台词，重载 OBS 舞台后约 1.21 秒能再次看到人物。Vidu 确认移除道具后，杯沿和手部还留了约 20 秒残影。[人物照片](https://www.metmuseum.org/art/collection/search/267426) · [道具照片](https://www.metmuseum.org/art/collection/search/49831)

联测里有本机测试员通过弹幕入口提问，录像中录到了自动回复。还需要用正式角色和目标直播间的真实弹幕验收，并完成一次 30 分钟公网实播。

## 从源码构建

先装好 Xcode Command Line Tools、Rust 稳定版和 Python 3。在项目根目录运行下面的命令，会生成并打开 `dist/Avatar3D Live.app`。需要指定签名身份时，设置 `AVATAR3D_SIGN_IDENTITY`。

```sh
cargo test --locked
./scripts/build-macos.sh
open "dist/Avatar3D Live.app"
```

开发时可以直接跑 `cargo run --locked`。数据默认放在 `~/Library/Application Support/Avatar3D Live`。可以用 `AVATAR3D_DATA_DIR` 改数据目录，用 `AVATAR3D_PORT` 改控制台端口。模型权重、导入的音频和上传的角色图都保存在本机。

Vidu 确认结束会话后，`sessions.jsonl` 会记下首帧时间、已发的 PCM 音频字节数和平台返回的结算字段。控制台显示的首段音频延迟，是从台词排队到第一段 PCM 进入发送队列的时间。观众实际什么时候听到声音，要从直播回放里测。

| 代码位置 | 职责 |
| --- | --- |
| `src/session.rs`、`src/vidu.rs`、`src/wire.rs` | 建立和结束 Vidu 会话、核对状态、收发 WebSocket 信令和发送 PCM |
| `src/speech.rs`、`src/audio.rs`、`src/dialogue.rs` | 回复排队、打断、分句生成和音频转换 |
| `src/audience.rs`、`src/library.rs`、`src/choreo.rs` | 处理弹幕、管理曲库和循环动作 |
| `src/obs.rs`、`src/output.rs`、`web/stage.js` | 创建 OBS 场景、分配舞台观看席位和接收 RTC 画面 |
| `web/`、`packaging/`、`scripts/` | 控制台界面、macOS 应用资源、构建和模型启动脚本 |
