# streaming-asr-server 开发指南

## 项目定位

基于 sherpa-onnx 的流式中文 ASR WebSocket 服务端。客户端通过 WebSocket 发送 PCM 16kHz 单声道音频，服务端实时返回 partial/final 识别结果。

技术栈：Rust + axum + sherpa-onnx + tokio。

## 常用命令

```bash
cargo run                        # 开发启动（默认 0.0.0.0:6008，无需 --model）
make dev                         # 同上，快捷别名
make test                        # 跑全部测试（含真实音频管道）
cargo test                       # 跑全部测试（不含输出）
cargo test <test_name> -- --nocapture  # 跑单个测试并显示输出
```

## 模型下载优先级

启动时自动检测模型目录，缺失则下载。URL 解析优先级：

1. `--model-url` CLI 参数（最高）
2. `$ASR_MODEL_URL` 环境变量
3. 默认 GitHub URL（中文 zh-xlarge-int8 模型，~700MB）

模型目录默认 `$HOME/.cache/asr-server/models/`，可通过 `--model` 自定义。

支持三种协议：
- `https://` / `http://` — 流式下载 .tar.bz2
- `file://` — 直接提取本地压缩包（不解压时需自行放置 4 个文件）

代理自动探测：`HTTPS_PROXY` > `HTTP_PROXY` > `ALL_PROXY`。

## 模型文件要求

目录内必须有且仅有 4 个文件，文件名固定：

```
encoder.int8.onnx   decoder.onnx   joiner.int8.onnx   tokens.txt
```

由 `src/model.rs` 的 `MODEL_FILES` 常量定义，与 sherpa-onnx zipformer-transducer 架构绑定。

## 架构要点

```
main.rs → ws_handler.rs → asr.rs → sherpa-onnx
              ↑              ↑
         protocol.rs    session.rs (Semaphore)
         auth.rs        config.rs → model.rs
```

- **OnlineRecognizer** 全进程共享，各 WS 会话独立 `OnlineStream`（sherpa-onnx 原生支持多 stream）
- **i16 → f32 归一化**：`src/asr.rs:accept_waveform` 中 `s as f32 / 32768.0`
- **Unsafe reinterpret cast**：`src/ws_handler.rs` 中 `from_raw_parts` 将 `&[u8]` 转为 `&[i16]`（LE 架构安全，BE 会 UB）
- **Semaphore lifetime**：`SessionGuard` 必须移入 `ws.on_upgrade` 闭包随会话持有，否则槽位立即释放（d75ae 修复）

## WS 协议

`src/protocol.rs` 定义全部帧类型。Client→Server 三种：`start` / `finish` / `ping`。Server→Client 六种：`ready` / `listening` / `partial` / `final` / `pong` / `error`。

连接复用时 `finish` 后回到 `ready`，不关闭连接，可重复 `start→finish`。

空闲 60 秒无帧 → 服务端发 fatal error 断开。

## 鉴权与并发顺序

当前顺序：**先检查槽位，后检查鉴权**。这意味着未认证连接会消耗并发槽位，可被 DoS。如需修复应调换顺序。

两种鉴权方式（`src/auth.rs`）：
- `Authorization: Bearer <token>`
- `X-ASR-Token: <token>`

服务端未配置 `--auth-token` 时跳过鉴权。

## 测试说明

10 个测试在 `src/main.rs` 底部 `#[cfg(test)]` 模块中，每个测试启动内嵌 server 绑定随机端口。测试依赖真实 sherpa-onnx 库（编译期自动下载），需要模型文件在 `$HOME/.cache/asr-server/models/` 就绪。

`audio_real_file_pipeline` 使用 `tests/fixtures/zh-test.pcm`（中文真实音频）。

## 待清理

- `sha2`、`hex`、`toml` 声明但未使用，可移除
- `tests/fixtures/en-test.pcm` 存在但无测试引用
- 缺少 SIGTERM 优雅退出

## 部署

```bash
# 本地启动（模型自动下载）
asr-server

# 生产示例
asr-server \
  --bind 0.0.0.0:6008 \
  --tls-cert /etc/asr/cert.pem \
  --tls-key /etc/asr/key.pem \
  --auth-token "$(cat /etc/asr/token)" \
  --max-sessions 2 \
  --num-threads 4
```

## 文档

客户端接入文档见 [docs/client-api.md](docs/client-api.md)。
