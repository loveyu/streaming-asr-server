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

HTTP 升级请求时携带对应 Header。鉴权失败返回 HTTP `401`，响应体 JSON：

```json
{"error":"auth","code":"auth","message":"Unauthorized","fatal":true,"retry":false}
```

`code` 为 `auth`、`fatal:true`，客户端应据此提示"请检查远端 Token 配置"，而非静默回退本地。

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
{"type":"start","sample_rate":16000,"idle_seconds":30}
```
- `sample_rate` 可选，默认 16000。客户端按实际音频采样率传递，服务端自动适配。支持 8000/16000/22050/44100/48000 等常见采样率。
- `idle_seconds` 可选，客户端建议的本轮空闲超时秒数。服务端按需采纳（钳制到 5~600s）。
  未提供时使用服务端默认（`--idle-timeout`，默认 60s）。

收到后服务端重置识别状态，回复 `listening`。若上一轮**未 `finish` 且有已识别文本**就再次 `start`，服务端会先补发上���轮的 `final` 再开始新轮，避免已识别内容丢失；若上一轮已 `finish` 或无文本，则**静默重置**，新轮首个下发帧为 `listening`（不补发空 `final`）。

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

`text` 为当前累计识别文本，会随新音频数据不断更新增长。`partial` 仅在 `text` **非空且与上一次不同**时下发（静音段不下发空 `partial`）。

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

**错误通知（结构化，R2）：**

```json
{
  "type": "error",
  "code": "idle",
  "message": "idle timeout",
  "fatal": false,
  "retry": true
}
```

| 字段 | 含义 |
|------|------|
| `code` | 错误分类：`idle` / `connection` / `auth` / `internal` / `overload` / `protocol` |
| `fatal` | `false` 可恢复，连接保持；`true` 服务端即将关闭连接 |
| `retry` | 客户端是否可重试/重连 |

`fatal` 语义：仅 `connection`（链路断）、`auth`、`internal` 为 fatal；`idle`、`overload`、`protocol` 等可恢复。

> 空闲超时（`code:"idle"`）属于业务级结束，**不是** `fatal`。客户端不应据此判定远端不可用或回退本地。

## 空闲超时

服务端默认 60s 内无任何帧（含 `ping`/音频）即视为本轮结束（`--idle-timeout` 可配，也可在 `start` 中用 `idle_seconds` 按轮覆盖）。触发时服务端：

1. 补发本轮已识别的 `final`（含已识别文本，可为空，R1/R3）；
2. 发送非致命 `error`（`code:"idle"`，`fatal:false`，`retry:true`）；
3. 发送 WebSocket `close` 帧优雅关闭（R3）。

因此**不会**出现"用户停顿即被判远端失败"。建议客户端仍每 ~30s 发一次 `{"type":"ping"}` 以防中间代理断开空闲连接；服务端还会每 15s 主动发 WebSocket `Ping` 探活并清理半开连接（R3）。

## 错误码与场景

| 场景 | HTTP / WS | `code` | `fatal` | `retry` | 行为 |
|------|-----------|--------|---------|---------|------|
| 鉴权失败 | HTTP `401` + JSON | `auth` | true | false | 关闭连接，提示检查 Token |
| 槽位满 | HTTP `503` + JSON | `overload` | false | true | 关闭连接，可退避重试/降级 |
| 未 start 发送音频 | WS `error` | `protocol` | false | false | 连接保持 |
| 非法 JSON | WS `error` | `protocol` | false | false | 连接保持 |
| 空闲超时 | WS `final` + `error` | `idle` | false | true | 补发 final 后优雅关闭 |
| 链路异常 | WS `final` + `error` | `connection` | true | true | 尝试补发 final 后关闭 |
| 客户端正常断连 | WS `close` | — | — | — | 释放槽位 |

## 协议健壮性实现状态

服务端已实现需求文档中的全部服务端项（R1–R6 及 N1/N2）：

- **N1**：复用连接再次 `start` 时不再下发空 `final`——上一轮无文本则静默重置（`build_final` 对空文本返回 `None`），新轮首帧为 `listening`；仅当上一轮有非空未提交文本时才补发 `final`。
- **N2**：`partial` 仅在文本非空且与上一次不同时下发（`last_partial` 去重），静音段不再产生空 `partial`。

剩余 **N3**（解析 HTTP 升级失败、区分鉴权/满载）属客户端侧（插件 `RemoteSpeechRecognizer.WsHandler.onFailure`），服务端已提供同形 JSON 体（401→`code:auth`、503→`code:overload`）供其解析。

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
