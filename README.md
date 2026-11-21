# Avatar3D Live

Avatar3D Live是 Apple Silicon Mac 上的角色互动直播控制台。观众发来弹幕后，操作员可以让角色用中文回应、打断播音或触发动作，再用 OBS 推流。当前版本将单人角色图或 Vidu 形象 ID 交给 Vidu S2 生成实时视频。本机负责对话、语音合成、动作调度和输出。VRM、GLB 模型加载和骨骼渲染尚未实现。[来源与授权](NOTICE.md) · [MIT 许可证](LICENSE)

## 在 Mac 上打开

双击「Avatar3D Live.app」。应用会在默认浏览器打开控制台，后台进程仅监听本机。再次打开应用可以找回控制台；要退出后台进程，使用「设置 → 退出 Avatar3D」。此版本在 Apple M4 上构建与验证，采用本地签名。新机器需要 FFmpeg，运行本机模型还需要 uv。使用 Homebrew 可以执行 `brew install ffmpeg uv`，推流使用 OBS Studio 28 或更新版本。[OBS 下载](https://obsproject.com/download)

在「设置」中点「启动本机 MLX」和「启动本机对话」。首次启动会安装各自的 Python 环境并下载模型，看到两个「已就绪」后即可试听。本机语音使用 Ming 3.0 0.5B 4-bit 的灵小柔和 ZONOS3-Mini 的 Willow，纯英文使用 Willow，含中文的句子使用灵小柔。灵小柔使用 MLX 编译和五步推理。对话默认使用 Qwen2.5-1.5B-Instruct 4bit。也可以填写兼容服务接口，参考录音需要该服务支持并提供逐字稿。[MLX Audio](https://github.com/Blaizzy/mlx-audio) · [MLX LM](https://github.com/ml-explore/mlx-lm)

## 先排练，再连线

1. 在「直播台 → 直接说话」输入自己的台词，点「本机试听」。切到「AI 回复」并试听可以检验完整的本机问答和合成流程。这两项均可在 Vidu 待机状态使用。
2. 在 OBS 的「设置 → 视频」选择需要的画布与输出尺寸，例如竖屏 720 × 1280。在「工具 → WebSocket 服务器设置」启用服务器。在 Avatar3D Live「设置 → OBS 场景」填写本机端口及密码，点「建立 Avatar3D 场景」，然后在 OBS 选中「Avatar3D · 数字人」。
3. 回到 Avatar3D Live，确认舞台显示 WebRTC 与 H.264 可用。开启 OBS 本地录制，点「舞台试音」，在 OBS 混音器和录制回放中检查电平与声音。也可以把「复制 OBS 地址」所得地址手工加入浏览器源，并勾选通过 OBS 控制音频。独立舞台适合浏览器排练，同时只有一个输出端占用 RTC 观看席位。切换到 OBS 前请关闭独立舞台。
4. 在「设置」保存 Vidu 密钥，上传自己的单人角色图或填写账户内的形象 ID。支持 PNG、JPEG、WebP，图片上限 8 MiB，已填 ID 时优先使用 ID。选定分辨率、连线时限与所需动作库，点「保存设置」。默认时限为 5 分钟。
5. 点「开始连线」核对时限和估算费用。信令就绪后输入台词或动作，观察舞台首帧、人物身份、口型、音画同步与 OBS 录制。OBS 面向观众的推流由操作员在 OBS 中启动和停止。直播结束时还需在 Avatar3D Live 点「结束连线」，确认显示「Vidu 已确认会话结束」。

Vidu 组件版公开价为每秒 1 积分，折合每分钟 1.875 元。30 分钟约需 1,800 积分。时限触发后应用发起挂断，实际结算以平台为准。连接中断只尝试恢复原场次，无法查明创建或结束结果时保留待核对记录。应用重启后可以在控制台继续核对。[Vidu S2 参数与计费](https://platform.vidu.cn/vidu-stream/doc/s2-avatar/component/parameters)

## 弹幕、自动播报与曲库

在「设置 → 语音与直播环境」中单选**中文直播**或**英文直播**，本机 MLX 仅加载当前语言对应的单个 TTS 模型（中文用灵小柔、英文用 Willow），两者不会共存。自动欢迎、礼物答谢和歌后谢幕按场景选择话术。明确指定的 QA 回复和跳过规则优先执行，需要结合弹幕或礼物留言临场接话时使用语言模型。播报依然按设定的最小间隔串行执行。

在「设置 → 编辑话术」可以查看并保存当前语言的档案。档案位于应用数据目录下的 `prompts/<档案名>/zh.json` 或 `en.json`。首次加载会沿用已保存的人设和欢迎词。如果仓库里已有 `avatars/cn/xiaorou_knowledge_base.txt`，中文档案会直接引用该文件，文件修改后自动生效。设置页显示生效版本，保存时会检查磁盘版本。

`events` 中每个场景可填写多条完整句子，实时选句会避开近期播过的版本。`chat_rules` 可以匹配指定弹幕，绑定原 QA 答案、设置跳过或提供多条回复。`gift_rules` 可以按真实礼物名称和数量绑定 QA 答案。只有可信的同一观众追问会带入最近两轮上下文。

不唱歌时，系统会按间隔轮换参考欢迎句暖场，并在冷却时间满足时偶尔欢迎进房观众（中文模式下自动跳过含英文字母的用户名）。定时唱歌或弹幕点歌期间会自动暂停欢迎与编排动作，并暂存期间收到的礼物，待整首歌唱完并恢复自然神态后再统一答谢观众与礼物（中文模式下对英文用户名仅称呼老板/朋友，不念英文名）。

YouTube 使用 YouTube Data API v3。在「设置」保存 API Key 和正在直播的视频 ID，选择仅显示、自动朗读或自动回复。抖音、TikTok 等平台可用有授权的本机采集工具，向「设置 → 复制本机弹幕接入信息」中的 HTTP 地址发送 JSON（`POST http://127.0.0.1:19848/api/ingest`，`Authorization: Bearer <接入令牌>`）：

| 字段 | 内容 |
| --- | --- |
| `id` | 平台的稳定事件 ID，用于过滤重发 |
| `platform` | 平台标识：`douyin`、`tiktok`、`youtube` 或 `local`（可省略） |
| `kind` | 事件类型：`chat`（弹幕）、`gift`（礼物）、`join`（进房）；省略时按 `gift` 字段兼容 |
| `author` | 实际观众的显示名 |
| `viewer_id` | 平台提供的稳定观众 ID，用于同一观众的追问，可省略 |
| `text` | 弹幕原文或礼物留言（`join` 以及已填 `gift_name` 的 `gift` 可留空） |
| `gift` | 兼容旧格式：传 `true` 时按礼物处理 |
| `gift_name` | 礼物名称（如 `玫瑰`、`Rose`、`Super Chat`） |
| `gift_count` | 礼物数量，默认为 `1` |

「音频与参考图」支持播放音频、给出手持物或服装参考、切换背景及复位姿态。在设置中每行填写一条动作，可在连线后启动循环编排。曲库只收录手动导入的授权文件，每条不超过 32 MiB、10 分钟。启用点播后，观众发送「点歌」与完整曲名即可排入播放队列；开启定时自动唱歌后，也会按设定间隔自动轮播曲库。

## 验证范围

把实际录制文件的路径传给 `python3 scripts/inspect-recording.py`，可以逐帧核对解码帧数、平均帧率、最大帧间隔，以及每条音轨相对视频的起止时间差。正值表示音轨较晚。录制的时间戳能定位时间线上的间隔，口型则需要回看角色发音时的嘴部动作。应用包的 `Contents/Resources/inspect-recording.py` 也提供同一工具。

已在 Apple M4、macOS 26.6.2、OBS 32.0.4 上验证本机中文对话和语音合成、控制台播放、OBS 建场与录制，以及 OBS 内置浏览器的 WebRTC / H.264 解码能力。独立 OBS 配置还向本机 RTMP 接收端连续推流 1,805.333 秒。接收端解出 54,160 帧，画幅 720 × 1280，平均 30 fps。画面使用项目图标，7 段中文试音在接收文件中均可检出。

使用公版单人照片完成了 Vidu S2 五分钟私下联测。首帧耗时 33.6 秒，OBS 录像长 5 分 3 秒，Vidu 确认计费 264 秒。录像中能看到人物开口、转头、挥手和双手持杯。打断长句后约 2.25 秒听到新台词，重载 OBS 舞台后约 1.21 秒重新看到人物。云端确认移除道具后，杯沿和手部仍有约 20 秒残影。[人物照片](https://www.metmuseum.org/art/collection/search/267426) · [道具照片](https://www.metmuseum.org/art/collection/search/49831)

本场由本机测试员通过弹幕入口提问，录像中收到了自动回复。正式角色的保真度、目标直播间的观众弹幕和 30 分钟公网实播仍待验收。

## 从源码构建

安装 Xcode Command Line Tools、Rust 稳定工具链和 Python 3。在项目根目录运行以下命令，输出为 `dist/Avatar3D Live.app`。设置 `AVATAR3D_SIGN_IDENTITY` 可以选择自己的签名身份。

```sh
cargo test --locked
./scripts/build-macos.sh
open "dist/Avatar3D Live.app"
```

开发时可运行 `cargo run --locked`。使用 `AVATAR3D_DATA_DIR` 和 `AVATAR3D_PORT` 可指定独立的数据目录和控制台端口。默认数据目录为 `~/Library/Application Support/Avatar3D Live`。凭据由 macOS 钥匙串保管，模型权重、导入音频和用户角色图存于各自设备。云端确认结束后，`sessions.jsonl` 记录首帧时间、已送出的 PCM 字节数和平台实际返回的结算字段。控制台的首段音频延迟从排队开始计到首段 PCM 进入发送队列，观众端听到声音的延迟需通过直播回放测量。

| 代码位置 | 职责 |
| --- | --- |
| `src/session.rs`、`vidu.rs`、`wire.rs` | 云端场次、状态核对、WebSocket 信令与 PCM 发送 |
| `src/speech.rs`、`audio.rs`、`dialogue.rs` | 回复队列、打断、分句生成与音频转换 |
| `src/audience.rs`、`library.rs`、`choreo.rs` | 实时弹幕、授权曲库、动作编排 |
| `src/obs.rs`、`output.rs`、`web/stage.js` | OBS 场景、舞台独占、RTC 接收与实际画面监看 |
| `web/`、`packaging/`、`scripts/` | 操作界面、macOS 应用资源、本机构建与模型启动 |
