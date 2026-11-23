# 开发与打包

## 从源码构建

先装好 Xcode Command Line Tools、Rust 稳定版和 Python 3。在项目根目录运行下面的命令，会构建当前 Mac 架构的应用，校验签名后打开控制台。

```sh
cargo test --locked
./scripts/build-macos.sh
open "dist/Vorela.app"
```

要分发应用，运行 `./scripts/build-macos.sh --zip`，将生成的 `dist/Vorela.zip` 发给使用者，解压后即可将 App 拖入「应用程序」。脚本最后一个参数可指定输出目录。默认使用本地签名，也可以用 `VORELA_SIGN_IDENTITY` 指定签名身份。

应用包带上项目里已有的中英文角色知识库和曲名映射。移出源码目录后，首次运行会将缺少的文本补入应用数据目录的 `avatars/` 下，此后可以直接编辑。独立安装的 App 从本机应用数据目录读取 `.env`，也可以在「设置」里填写自己的密钥。在仓库里运行 `dist/` 下的 App 时，仍沿用项目的角色文件和密钥。

开发时运行 `cargo run --locked` 会打开同一套桌面窗口。服务模式使用 `cargo run --locked -- --no-open`。控制台、OBS 舞台和弹幕接口共用仅本机可访问的端口，用 `VORELA_PORT` 可以修改。数据默认放在 `~/Library/Application Support/Vorela`，用 `VORELA_DATA_DIR` 可以修改。模型权重、导入的音频和上传的角色图都保存在本机。

Vidu 确认结束会话后，`sessions.jsonl` 会记下首帧时间、已发的 PCM 音频字节数和平台返回的结算字段。控制台显示的首段音频延迟，是从台词排队到第一段 PCM 进入发送队列的时间。观众实际什么时候听到声音，要从直播回放里测。

## 改一个功能，从哪里开始

| 要改的行为 | 从这里读 |
| --- | --- |
| 启动、退出、打开窗口 | `src/main.rs` 启动本机 Axum 服务；`src/desktop.rs` 调用 `native/desktop/` 的 AppKit 窗口。 |
| 控制台、设置、舞台 | `web/index.html` 和 `web/settings.html` 组成控制台，`web/stage.html` 是独立舞台。`src/server.rs` 将页面、脚本和样式嵌入可执行文件，修改后重新编译。 |
| 收弹幕、点歌、礼物答谢 | `src/douyin/` 和 YouTube 接收端将消息送到 `src/audience.rs`；后者按事件类型交给 `src/speech.rs` 排队。演唱流程在 `src/speech/song.rs`。 |
| Vidu 连线与 OBS 推流 | `src/session.rs` 管理连线生命周期，`src/vidu.rs` 和 `src/wire.rs` 对接 Vidu。`src/obs.rs` 操作 OBS，`web/stage.js` 接收 RTC 画面。 |
| HTTP 接口与设置 | `src/api.rs` 注册业务路由，`src/server.rs` 检查调用权限。`src/config.rs` 校验和保存设置，`src/paths.rs` 决定数据从哪里读。 |
| 话术、本机模型 | `prompts/defaults/` 保存中英文默认话术。`runtime/models/` 的脚本由 `src/services.rs` 嵌入并在模型启动时释放到数据目录。 |
| 打包 | `scripts/build-macos.sh` 构建并签名 App，图标和 Info.plist 在 `packaging/`。 |

## 提交前检查

```sh
cargo fmt --all -- --check
cargo test --locked
python3 scripts/check-i18n.py
```

Rust 单元测试随模块存放。抖音模块另有 4,006 条上游 Go 输出作为对照，更新步骤见[录制说明](https://github.com/JASON-QWeb/Live3D/blob/main/scripts/douyin/README.md)。

`target/` 是 Cargo 缓存，`dist/` 是发布产物。本机的 `.env`、角色素材 `avatars/`、设计过程文件 `design/` 和录制输出 `output/` 由 Git 忽略。打包时只有构建脚本列出的角色文本会被选入；同一提交在没有这些可选文本的机器上仍能构建。
