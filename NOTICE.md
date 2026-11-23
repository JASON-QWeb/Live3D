# 来源与授权

本仓库的 Rust 会话与音频调度、直播控制台、舞台页、OBS 接口和启动脚本由项目贡献者编写，按 MIT 许可证发布。Vidu 适配器依据服务商公开的 S2 组件版接口实现。[项目许可证](LICENSE) · [Vidu S2 接口文档](https://platform.vidu.cn/vidu-stream/doc/s2-avatar/component/parameters)

Vidu 云端数字人服务由用户使用自己的账户接入，计费和使用范围遵循开放平台条款。使用者负责取得所用角色图、真人肖像、音色、参考图和音乐的相应授权，并按授权范围使用录播。[Vidu 用户协议](https://platform.vidu.cn/docs/terms-and-policies/user-policy)

舞台在需要时获取腾讯 TRTC Web SDK 5.21.0，校验 SHA-384 后保存在本机数据目录。SDK 及其服务使用条件由腾讯提供。OBS 通过公开的 obs-websocket v5 接口建立浏览器源。[TRTC 文档](https://web.sdk.qcloud.com/trtc/webrtc/v5/doc/en/TRTC.html) · [OBS WebSocket 协议](https://github.com/obsproject/obs-websocket/blob/master/docs/generated/protocol.md)

本机模型通过单独的 Python 环境下载。Ming、ZONOS 与 Qwen 的权重、所选音色、分词器与运行库分别遵循各自分发的条款。使用自己的克隆音色时，应取得对应声音及录音的授权。macOS 系统语音用于个人试听，公开播出请选择具备相应使用权的语音来源。[Ming 4-bit 模型卡](https://huggingface.co/jorgepb96/Ming-3.0-tts-0.5B-4bit) · [ZONOS3-Mini 模型卡](https://huggingface.co/mlx-community/ZONOS3-Mini) · [Qwen 模型卡](https://huggingface.co/mlx-community/Qwen2.5-1.5B-Instruct-4bit) · [MLX Audio](https://github.com/Blaizzy/mlx-audio) · [MLX LM](https://github.com/ml-explore/mlx-lm) · [Apple 软件许可](https://www.apple.com/legal/sla/)

抖音弹幕连接（`src/douyin/` 的签名、参数拼接、页面解析和消息格式）移植自 douyinLive 的 commit af34f7c，原作者今晚打打，MIT 许可证。protobuf 字段编号取自 douyinlive-proto v0.1.3。`src/douyin/recorded/` 里的对照数据是用 rust-anything 记录的 douyinLive 原版输出。这些都不是抖音官方接口，使用时应遵守抖音的用户协议。[douyinLive](https://github.com/jwwsjlm/douyinLive) · [douyinlive-proto](https://github.com/jwwsjlm/douyinlive-proto) · [rust-anything](https://github.com/erkamyaman/rust-anything)

Rust 依赖版本由 Cargo.lock 固定。打包脚本按当前构建平台收集依赖的许可证正文，随应用存放于 Contents/Resources/ThirdParty.txt。

## douyinLive

MIT License

Copyright (c) 2025 今晚打打

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
