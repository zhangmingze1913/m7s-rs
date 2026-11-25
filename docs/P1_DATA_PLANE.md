# P1 数据面设计

## 1. 当前瓶颈

5000 路 HTTP-FLV 时平均 CPU 92.2%，主要成本：

1. `Stream::push` 对每个 `broadcast::Sender` 接收者做一次 `Arc` 克隆/唤醒；
2. 5000 个接收者 = 每帧 5000 次广播投递；
3. `RwLock<VecDeque>` GOP 缓存在 push/subscribe 时竞争；
4. 每个 viewer 独立 task + 独立写 socket；
5. 无零拷贝：FLV tag 是 `Arc<FlvTag>`，payload 仍是 `Vec<u8>`；
6. 无背压策略：`Lagged` 直接丢，慢消费者无法请求关键帧。

10K 失败不是单点 bug，而是 fan-out 模型和任务模型到达上限。

## 2. 目标模型

```text
Publisher
   ↓ 一次写入
MediaTrack (video/audio)
   ├─ FrameRing (固定容量, 覆盖旧帧)
   ├─ ArcSwap<GopCache> / ArcSwap<Metadata>
   └─ Notify (single wake, all subscribers)
        ↓
Subscriber (per viewer)
   ├─ cursor: u64
   ├─ read frames >= cursor
   ├─ lag → seek 到最近关键帧
   └─ write socket / muxer
```

关键点：

- **一次发布，多次读取**：不再每帧向 N 个 broadcast 接收者 send；
- **subscriber cursor**：每个 viewer 自己推进，不复制队列；
- **固定容量 ring**：覆盖旧帧，内存有界；
- **lag 检测**：cursor 落后于 ring 起点时跳到最近关键帧；
- **关键帧请求**：慢消费者/新消费者请求 publisher 产生 IDR；
- **零拷贝**：`Bytes`/`Arc<[u8]>` 共享 payload，muxer 只写 header/分片。

## 3. 数据结构

```rust
pub struct MediaFrame {
    pub track: TrackKind,      // Video/Audio/Metadata
    pub sequence: u64,         // per-track monotonic
    pub timestamp_ms: u32,
    pub keyframe: bool,
    pub data: Arc<[u8]>,       // zero-copy payload
}

pub struct FrameRing {
    slots: Box<[UnsafeCell<Option<Arc<MediaFrame>>>]>,
    // 生产端单写，消费端 cursor + seqlock/epoch
}
```

实现选择：

1. 第一版可用 `crossbeam_epoch` 保护 `Arc` 回收，避免 seqlock UB；
2. 或使用 `ArcSwap<VecDeque<Arc<MediaFrame>>>` 做不可变快照，读无锁、写一次替换；
3. 不直接手写 `UnsafeCell` seqlock，除非有 Loom/Miri 证明。

## 4. 迁移步骤

### P1.1 统一模型

- 新增 `MediaFrame/TrackKind`；
- `FlvTag` → `MediaFrame` 转换；
- RTMP/FLV/HLS/RTSP 逐步改用 `MediaFrame`；
- 保留 FLV tag 兼容层。

### P1.2 发布/订阅

- `Stream` 移除 `broadcast::Sender`；
- `publish(frame)` 写入 ring + `Notify::notify_waiters()`；
- `subscribe()` 返回 `Subscriber { cursor }`；
- `Subscriber::next()` 读取 ring，落后则 seek 关键帧；
- 所有 `rx.recv()` 改为 `subscriber.next()`。

### P1.3 零拷贝

- payload 用 `Arc<[u8]>`/`Bytes`；
- HLS/FLV muxer 写 `&[u8]`，不复制 NAL；
- GOP cache 只存 `Arc`。

### P1.4 背压

- ring 满覆盖；
- subscriber lag 超过阈值 → 跳到最近关键帧；
- 支持 `request_keyframe()`（向 publisher 发信号）。

### P1.5 workspace

```text
crates/
  codec/       FLV/H264/AAC/TS
  core/        MediaFrame/FrameRing/Track
  protocol/    RTMP/RTSP/HLS/HTTP-FLV
  service/     StreamManager/Cluster/Admin
  app/         main
```

## 5. 验收

- 单流 10K 订阅（合成流）通过；
- 5000 路 CPU 明显低于当前 92.2%；
- P50/P99 帧延迟报告；
- 慢消费者 lag 后能恢复关键帧；
- 无 UB：Loom/Miri/ASan（Linux）；
- 与当前实现同机对比。

## 6. 当前进度

- P1.1 `MediaFrame/TrackKind`：已实现；
- P1.2 `MediaRing/MediaSubscriber`：已实现模型与单测；
- 协议路径保留 `broadcast`；HTTP-FLV 写路径改为 64KB 批量聚合；
- 零拷贝/背压/关键帧请求：未实现。

## 7. 不做

- 不先写 WebRTC/SRT；
- 不先做 26 插件；
- 不在没有 P99/10K 证据时宣称 V6 对标。
