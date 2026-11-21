# 来源与授权

本仓库的 Rust 会话与音频调度、直播控制台、舞台页、OBS 接口和启动脚本由项目贡献者编写，按 MIT 许可证发布。Vidu 适配器依据服务商公开的 S2 组件版接口实现。[项目许可证](LICENSE) · [Vidu S2 接口文档](https://platform.vidu.cn/vidu-stream/doc/s2-avatar/component/parameters)

Vidu 云端数字人服务由用户使用自己的账户接入，计费和使用范围遵循开放平台条款。使用者负责取得所用角色图、真人肖像、音色、参考图和音乐的相应授权，并按授权范围使用录播。[Vidu 用户协议](https://platform.vidu.cn/docs/terms-and-policies/user-policy)

舞台在需要时获取腾讯 TRTC Web SDK 5.21.0，校验 SHA-384 后保存在本机数据目录。SDK 及其服务使用条件由腾讯提供。OBS 通过公开的 obs-websocket v5 接口建立浏览器源。[TRTC 文档](https://web.sdk.qcloud.com/trtc/webrtc/v5/doc/en/TRTC.html) · [OBS WebSocket 协议](https://github.com/obsproject/obs-websocket/blob/master/docs/generated/protocol.md)

本机模型通过单独的 Python 环境下载。Ming、ZONOS 与 Qwen 的权重、所选音色、分词器与运行库分别遵循各自分发的条款。使用自己的克隆音色时，应取得对应声音及录音的授权。macOS 系统语音用于个人试听，公开播出请选择具备相应使用权的语音来源。[Ming 4-bit 模型卡](https://huggingface.co/jorgepb96/Ming-3.0-tts-0.5B-4bit) · [ZONOS3-Mini 模型卡](https://huggingface.co/mlx-community/ZONOS3-Mini) · [Qwen 模型卡](https://huggingface.co/mlx-community/Qwen2.5-1.5B-Instruct-4bit) · [MLX Audio](https://github.com/Blaizzy/mlx-audio) · [MLX LM](https://github.com/ml-explore/mlx-lm) · [Apple 软件许可](https://www.apple.com/legal/sla/)

Rust 依赖版本由 Cargo.lock 固定。打包脚本按当前构建平台收集依赖的许可证正文，随应用存放于 Contents/Resources/ThirdParty.txt。
