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
```

环境变量：
- `ASR_MODEL_URL` — 覆盖模型下载地址（优先级低于 `--model-url`）
- `HTTP_PROXY` / `HTTPS_PROXY` / `ALL_PROXY` — 模型下载代理

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
