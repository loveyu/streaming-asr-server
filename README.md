# streaming-asr-server

基于 [sherpa-onnx](https://github.com/k2-fsa/sherpa-onnx) 的**流式中文语音识别 WebSocket 服务端**。

客户端发送 PCM 16kHz 音频流，服务端实时返回 partial/final 识别结果。

## 快速开始

```bash
# 首次启动会自动下载模型（~700MB）到 ~/.cache/asr-server/models/
asr-server

# 或指定模型目录（跳过下载）
asr-server --model /path/to/sherpa-onnx-model

# 使用本地压缩包（不解压）
asr-server --model-url "file:///path/to/model.tar.bz2"
```

服务默认监听 `0.0.0.0:6008`，模型未就绪时自动下载。

## 配置

```
asr-server [OPTIONS]

  --model <DIR>                模型目录 [默认: ~/.cache/asr-server/models/]
  --model-url <URL>            模型下载地址（http/file://），优先级高于 $ASR_MODEL_URL
  --bind <ADDR>                监听地址 [默认: 0.0.0.0:6008]
  --auth-token <TOKEN>         鉴权 Token（不提供则跳过鉴权）
  --max-sessions <N>           并发上限 [默认: 2]
  --num-threads <N>            ONNX 推理线程数 [默认: CPU 核数]
  --decoding-method <METHOD>   解码方法 [默认: greedy_search]
  --max-active-paths <N>       Beam search 路径数 [默认: 4]
  --tls-cert <PATH>            TLS 证书（启用 WSS）
  --tls-key <PATH>             TLS 私钥
  --endpoint-silence <SEC>     端点静音阈值 [默认: 1.2]
  --endpoint-max-utterance <SEC> 单句最长时长 [默认: 20.0]
  --sample-rate <HZ>           音频采样率 [默认: 16000]
  --idle-timeout <SEC>         单轮空闲超时（也可在 start 用 idle_seconds 按轮覆盖）[默认: 60]

  --log-level <LEVEL>          日志等级：trace/debug/info/warn/error [默认: info]
  --log-file <PATH>            日志文件（路径或目录）[默认: 系统日志目录]
  --no-log-file                禁用文件日志，仅输出到 stderr
```

环境变量：
- `ASR_MODEL_URL` — 覆盖模型下载地址（优先级低于 `--model-url`）
- `HTTP_PROXY` / `HTTPS_PROXY` / `ALL_PROXY` — 模型下载代理
- `ASR_LOG` — 日志等级（优先级低于 `--log-level`，高于 `RUST_LOG`）
- `ASR_LOG_FILE` — 日志文件路径（优先级低于 `--log-file`）

## 日志

默认同时输出到 stderr 和一个文件。日志文件位置按优先级选取：

1. `--log-file` / `$ASR_LOG_FILE` 指定的路径（按字面使用）
2. 系统日志目录 `/var/log/asr-server/asr-server.log`（无写权限时自动回退）
3. 用户状态目录 `~/.local/state/asr-server/asr-server.log`
4. 临时目录（兜底）

日志等级默认 `info`，可用 `--log-level` / `$ASR_LOG` / `$RUST_LOG` 调整。`$ASR_LOG` 也接受
完整 `tracing` 指令，如 `streaming_asr_server=debug,sherpa_onnx=info`。文件日志为追加写入
（同步落盘，`SIGTERM` 不丢日志）；长期运行可用 logrotate 管理该文件。`--no-log-file` 关闭
文件输出。

## 客户端接入

详见 [docs/client-api.md](docs/client-api.md)。

简要流程：

```
Client                          Server
  ├── WS Connect ───────────────▶
  │◀── {"type":"status","state":"ready"}
  ├── {"type":"start"} ─────────▶
  │◀── {"type":"status","state":"listening"}
  ├── [PCM binary] ─────────────▶
  │◀── {"type":"partial","text":"今天"}
  ├── {"type":"finish"} ────────▶
  │◀── {"type":"final","text":"今天天气真不错",...}
  │◀── {"type":"status","state":"ready"}
```

音频要求：PCM 16-bit LE，16000Hz，单声道。

## 开发

```bash
make dev       # 启动开发服务器
make test      # 跑全部测试
cargo build    # 编译
```

详见 [AGENTS.md](AGENTS.md)。
