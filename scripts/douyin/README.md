# 重新录制 douyinLive 对照数据

`src/douyin/` 里的签名、参数拼接和页面解析移植自 [douyinLive](https://github.com/jwwsjlm/douyinLive) 的 commit af34f7c。`src/douyin/recorded/` 存着那一版 Go 代码在 4,006 个输入上的输出，`cargo test` 会拿 Rust 版逐条比对。

抖音改了签名或页面结构、douyinLive 跟着更新后，按下面的步骤录一次新版本的输出，就能找出 Rust 版要改的地方。需要 Go、Rust 和 [rust-anything](https://github.com/erkamyaman/rust-anything) 3.1。

1. 把 douyinLive 克隆到临时目录，并提交一次，方便之后看改动：

   ```sh
   export LIVE3D="$(pwd)"   # 从 Vorela 仓库根目录开始
   git clone https://github.com/jwwsjlm/douyinLive.git /tmp/dl && cd /tmp/dl
   git checkout af34f7c   # 更新对照数据时改用要验证的提交
   ```

2. 加上录制用的入口，只改这份临时副本：

   ```sh
   git apply "$LIVE3D"/scripts/douyin/ab_sign_at.patch
   cp "$LIVE3D"/scripts/douyin/golden_export.go .
   gofmt -w sign/ab_sign.go && go vet . ./sign/
   git add -A && git commit -m "recording wrappers"
   ```

   `ab_sign_at.patch` 让 a_bogus 能指定时间，原来的 `AbSign` 行为不变。`golden_export.go` 把根包里的私有函数包成可导出的函数，供 rust-anything 调用。上游改了这些私有函数的名字或参数时，要跟着改这个文件。

3. 为三个单元生成桥接 crate，换上本目录 `ports/` 里的 `port.rs`。先把里面的 `LIVE3D` 换成 Live3D 项目的路径：

   ```sh
   rust-anything bridge . internal/webcastsign/
   rust-anything bridge . sign/
   rust-anything bridge . golden_export.go
   for c in webcastsign sign golden-export; do
     sed "s#LIVE3D#$LIVE3D#" "$LIVE3D/scripts/douyin/ports/$c-rs.port.rs" > rust/$c-rs/src/port.rs
     grep -q md-5 rust/$c-rs/Cargo.toml || sed -i '' 's/^\[dependencies\]$/[dependencies]\nmd5 = { package = "md-5", version = "0.10" }/' rust/$c-rs/Cargo.toml
   done
   ```

   生成的 crate 要求 Rust 1.97。本机是更早的版本时，把各 `Cargo.toml` 里的 `rust-version` 改成本机版本。

4. 录制、编译、比对：

   ```sh
   python3 "$LIVE3D/scripts/douyin/make_inputs.py"
   rust-anything record rust/webcastsign-rs --inputs webcastsign.jsonl
   rust-anything record rust/sign-rs --inputs sign.jsonl
   rust-anything record rust/golden-export-rs --inputs golden.jsonl
   for c in webcastsign sign golden-export; do rust-anything build rust/$c-rs && rust-anything diff rust/$c-rs; done
   ```

   录完后要从记录里删掉两类每次运行结果都不同的用例：`sign-rs` 里 `"fn":"AbSign"` 的用例（用当前时间），`golden-export-rs` 里 msToken 参数为空的 `GoldenRoomEnterQuery` 用例（随机生成 msToken）。

5. `diff` 全部一致、`rust-anything gate rust/<crate>` 通过后，把三个 `cases.jsonl` 分别复制为 `src/douyin/recorded/webcastsign.jsonl`、`ab_sign.jsonl` 和 `params_page.jsonl`，再跑一次 `cargo test`。

联网部分（ttwid、直播页、web/enter、im/fetch、WebSocket 握手、心跳和重连）没法这样录制，每次都受抖音服务器的实时状态影响，只能连真实直播间测试。
