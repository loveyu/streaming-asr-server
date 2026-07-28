# ASR WebSocket 客户端接入文档

## 端点

```
ws://<host>:6008/          # 明文（开发）
wss://<host>:6008/         # 加密（生产，服务端配置 --tls-cert/--tls-key）
```

仅 `GET /` 一条路由，连接成功后升级为 WebSocket。

## 鉴权

服务端配置了 `--auth-token` 时必须鉴权，否则跳过。两种方式任选其一：

**方式一：标准 Bearer Token（推荐）**

```
Authorization: Bearer <token>
```

**方式二：自定义 Header**

```
X-ASR-Token: <token>
```

HTTP 升级请求时携带对应 Header。鉴权失败返回 `401 Unauthorized` 并关闭连接。

## 并发限制

服务端有并发槽位上限（默认 2，通过 `--max-sessions` 配置）。槽满时返回 HTTP `503`，响应体 JSON：

```json
{"error":"busy","message":"All ASR slots occupied"}
```

客户端收到 503 不应重试当前服务端，应降级到备用节点或本地模型。

## 音频格式

| 参数 | 值 |
|------|-----|
| 编码 | PCM 16-bit signed little-endian |
| 采样率 | 任意，通过 `start` 消息的 `sample_rate` 指定（默认 16000）|
| 声道 | 单声道 |
| 帧大小 | 无限制（建议 100ms × 采样率对应的字节数）|
| 传输方式 | WebSocket Binary 帧 |

## 协议流程

```
Client                          Server
  │                               │
  ├── WS Connect ────────────────▶│ 鉴权（可选），分配槽位
  │◀── {"type":"status", ────────┤
  │    "state":"ready"}           │
  │                               │
  ├── {"type":"start"} ──────────▶│ 新建识别会话
  │◀── {"type":"status", ────────┤
  │    "state":"listening"}       │
  │                               │
  ├── [binary audio] ────────────▶│ 持续发送 PCM 音频
  │◀── {"type":"partial", ───────┤ 流式识别结果
  │    "text":"...","segment":0}  │
  │       ...                     │
  │                               │
  ├── {"type":"finish"} ─────────▶│ 结束当前句
  │◀── {"type":"final", ─────────┤ 最终结果
  │    "text":"...","segment":0,  │
  │    "tokens":[...],            │
  │    "timestamps":[...]}        │
  │◀── {"type":"status", ────────┤ 回到就绪
  │    "state":"ready"}           │
  │                               │
  │    （可重复 start→finish）     │
  │                               │
  ├── WS Close ──────────────────▶│ 释放槽位
```

连接复用时 `finish` 后无需重连，直接再发 `start`。

## 消息格式

### Client → Server（Text JSON）

**开始新的识别句：**

```json
{"type":"start"}
{"type":"start","sample_rate":8000}
```
`sample_rate` 可选，默认 16000。客户端按实际音频采样率传递，服务端自动适配。支持 8000/16000/22050/44100/48000 等常见采样率。收到后服务端重置识别状态，回复 `listening`。

**结束当前句：**

```json
{"type":"finish"}
```
服务端 flush 剩余缓冲，返回 `final` 结果后回到 `ready`。

**心跳：**

```json
{"type":"ping"}
```
建议每 30 秒一发，防止中间代理断开空闲连接。服务端回复 `{"type":"pong"}`。

### Client → Server（Binary）

PCM 16-bit LE 16000Hz 单声道原始音频。帧大小不限，建议 100ms（3200 bytes）平衡延迟与效率。必须 `start` 之后才能发送，否则返回 error。

### Server → Client（Text JSON）

**状态通知：**

```json
{"type":"status","state":"ready"}
{"type":"status","state":"listening"}
```

**流式识别结果 (partial)：**

```json
{
  "type": "partial",
  "text": "今天天气",
  "segment": 0
}
```

`text` 为当前累计识别文本，会随新音频数据不断更新增长。

**最终识别结果 (final)：**

```json
{
  "type": "final",
  "text": "今天天气真不错",
  "segment": 0,
  "tokens": ["今","天","天","气","真","不","错"],
  "timestamps": [0.0, 0.32, 0.48, 0.64, 0.96, 1.12, 1.36, 1.6]
}
```

`tokens` 为分词结果，`timestamps` 为各 token 对应的起始时间（秒，float64），与 tokens 一一对应。

**心跳响应：**

```json
{"type":"pong"}
```

**错误通知：**

```json
{
  "type": "error",
  "message": "not listening, send start first",
  "fatal": false
}
```

- `fatal: false` — 可恢复，连接保持
- `fatal: true` — 服务端即将关闭连接

## 空闲超时

60 秒内无任何帧（包括 ping），服务端主动发送 `fatal error` 并关闭连接。建议至少每 30 秒发一次 `ping`。

## 错误码与场景

| 场景 | HTTP / WS | 行为 |
|------|-----------|------|
| 鉴权失败 | HTTP `401` | 关闭连接，不占槽位 |
| 槽位满 | HTTP `503` + JSON | 关闭连接，建议降级 |
| 未 start 发送音频 | WS `error`, `fatal:false` | 连接保持 |
| 非法 JSON | WS `error`, `fatal:false` | 连接保持 |
| 空闲超时 | WS `error`, `fatal:true` | 关闭连接 |
| 客户端断连 | — | 释放槽位 |

## 客户端参考实现

```python
import asyncio
import websockets
import numpy as np

async def asr_client(audio_pcm: bytes):
    async with websockets.connect(
        "ws://localhost:6008/",
        extra_headers={"Authorization": "Bearer my-token"}  # 鉴权
    ) as ws:
        # 等待 ready
        msg = await ws.recv()
        print(msg)

        # 开始识别
        await ws.send('{"type":"start"}')
        msg = await ws.recv()
        print(msg)  # listening

        # 发送 PCM（100ms 每块）
        chunk_size = 3200  # 100ms × 16000Hz × 2 bytes
        for offset in range(0, len(audio_pcm), chunk_size):
            chunk = audio_pcm[offset:offset + chunk_size]
            await ws.send(chunk)

            # 非阻塞读取 partial 结果
            try:
                msg = await asyncio.wait_for(ws.recv(), timeout=0.01)
                print(msg)
            except asyncio.TimeoutError:
                pass

        # 结束
        await ws.send('{"type":"finish"}')

        # 收取 final + ready
        while True:
            msg = await ws.recv()
            print(msg)
            if '"state":"ready"' in msg:
                break
```
