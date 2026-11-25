# OntoDB 调研与集成边界

## 1. 仓库状态

```text
gitee.com/ontovalue/ontodb
commit 13a33f7
version 0.6.2
license Apache-2.0
```

Rust workspace，包含：

- `onto-core` / `onto-storage` / `onto-ontology`
- `onto-query` / `onto-graph` / `onto-plugin`
- `onto-server` / `onto-cli` / `onto-edge`
- `ontodb-rules` / `onto-edge-esp32`

它是**本体驱动的多模态语义数据库**，不是流媒体协议栈，也不是 ONVIF 实现。

## 2. 与 m7s-rs 的关系

OntoDB 不适合放在 RTSP/RTP 热路径上：

- RTP/RTCP、HLS 切片、WebRTC 需要微秒/毫秒级内存数据面；
- OntoDB 是存储/查询/推理层，不是媒体转发层。

适合的位置是**控制面/元数据面**：

```text
m7s-rs 媒体面
  ├─ RTSP/RTP/RTCP
  ├─ RTMP/FLV/HLS
  └─ WebRTC/SRT/GB28181 (planned)
          │
          ▼ 设备/流/会话元数据
OntoDB（可选）
  ├─ ONVIF 设备目录、能力、厂商/型号
  ├─ GB28181 设备目录、行政区划、通道
  ├─ 流会话、录制索引、权限
  └─ 本体推理：设备类型/能力继承/互斥
```

## 3. 可落地的集成点（后续阶段）

1. **ONVIF 设备注册表**
   - `Device`、`Capability`、`Profile`、`Stream` 本体；
   - 查询“某厂商支持 H.264 且带 PTZ 的相机”。

2. **GB28181 目录**
   - 行政区划、设备类型、通道、父子关系；
   - `subClassOf` 自动展开设备类别。

3. **流会话/录制索引**
   - 流 ID、app、协议、起止时间、录制文件；
   - 与对象存储/本地文件索引关联。

4. **权限与租户**
   - 命名空间隔离；
   - 设备/流/用户的语义授权。

## 4. 明确不做

- 不用 OntoDB 替换 StreamManager/RingBuffer；
- 不在 RTP 包转发路径上同步写数据库；
- 不在当前 RTSP 互操作未闭环时引入新的存储依赖；
- 不把 OntoDB clone 包装成 ONVIF/GB28181 已实现。

## 5. 下一步

当前优先级仍是：

1. RTSP VLC 稳定播放；
2. GB28181（SIP + RTP/PS）；
3. ONVIF；
4. 之后再评估 OntoDB 作为设备/流元数据后端。

OntoDB 已 clone 到 `../ontodb`，仅完成代码结构/README 调研，未编译、未集成。
