<div align="center">

<img src="web/mark.png" width="96" height="96" alt="Vorela 图标" />

# Vorela

**在 Mac 上让数字人回弹幕，用 OBS 播出。**

[直播操作手册](docs/live-guide.md) · [开发与打包](docs/development.md) · [来源与授权](NOTICE.md)

</div>

## 能做什么

在控制台输入台词，或让数字人回复直播间的弹幕。对话和语音合成可以在 Apple Silicon Mac 上运行。Vidu S2 根据单人角色图或账户里的形象 ID 生成实时画面，OBS 负责录制和推流。当前版本不支持 VRM、GLB 或骨骼渲染。

- **接弹幕**　内置抖音直播间连接，支持 YouTube Data API 和本机事件接入。可以只看消息，也可以开启朗读、自动回复或礼物答谢。
- **排节目**　观众可从已导入的曲库点歌。唱歌期间收到的礼物会留到曲终答谢。舞台也能播放已导入的舞蹈视频。
- **先排练**　本机试听可以检查台词和回复。用 OBS 录一小段，再回放检查声音、口型和画面。

## 从源码运行

在 Apple Silicon Mac 安装 Xcode Command Line Tools、Rust 稳定版和 Python 3。音频处理需要 FFmpeg，使用本机模型还需要 uv。

```sh
brew install ffmpeg uv
cargo run --locked
```

要生成可双击的应用，在仓库根目录运行 `./scripts/build-macos.sh`，然后打开 `dist/Vorela.app`。打包分发和服务模式见[开发文档](docs/development.md)。

## 第一次开播

1. 在设置里启动本机 MLX 和本机对话服务。等模型就绪后，在直播台输入一句台词，点「本机试听」。
2. 打开 OBS Studio 28 或更新版本，启用 WebSocket 服务器。在 Vorela 的设置里点「建立 Vorela 场景」，再用「舞台试音」检查录制结果。
3. 保存 Vidu 密钥和角色图，点「开始连线」并核对时限和费用。需要推流时，在 OBS 设置栏填写直播平台提供的地址和推流码，点「开始直播」。

**下播后记得结束 Vidu 连线。** OBS 停止推流后，Vidu 会继续计费，直到你结束连线或达到设定时限。默认时限为 5 分钟，费用以平台结算为准。[直播操作手册](docs/live-guide.md#先排练再连线)

## 许可证

Vorela 使用 MIT 许可证。抖音协议移植、模型和 SDK 的来源见授权说明。[LICENSE](LICENSE) · [NOTICE.md](NOTICE.md)
