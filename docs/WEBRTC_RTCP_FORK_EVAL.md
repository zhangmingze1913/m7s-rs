# webrtc-rs RTCP fork 评估

Date: 2026-11-16

## 目标

让应用层拿到 REMB / TWCC 反馈，用于精确带宽估计与 RID 切层。

## 现状

- `webrtc 0.11` 内部处理 RTCP（NACK/PLI/REMB/TWCC），但未向应用暴露
  接收侧 RTCP 包或带宽估计。
- 现有自动切层只能用 `get_stats()` 的 `nack_count` 与
  `RemoteInboundRTP.round_trip_time`。
- 已实现 `parse_remb` 解析器，但无输入通道。

## 方案对比

| 方案 | 工作量 | 风险 |
|---|---|---|
| Fork webrtc-rs，暴露 RTCPReader/带宽估计 | 高（维护 fork） | API 漂移、升级成本 |
| Patch 依赖到本地 path，添加回调 | 中 | 每次升级需重放补丁 |
| 只用现有 stats（NACK/RTT） | 已完成 | 精度有限 |
| 在 UDP 层旁路抓 RTCP | 低但侵入 socket | 与 ICE/DTLS 冲突 |

## 推荐

短期保留 NACK/RTT；中期采用 **本地 patch + 上游 PR**：

1. 在 `PeerConnection` 增加 `on_rtcp` 回调；
2. 暴露 `RTCPReader::read`；
3. 将 REMB/TWCC 解析后回调给 WHIP；
4. 用带宽估计替代 NACK heuristic。

## 验收

- 回调收到 REMB bps；
- 自动层切换从 NACK/RTT 改为带宽阈值；
- 单元测试 + 真实 WHIP 会话验证。

未完成前，`/api/webrtc/capabilities` 保持 `remb:false, twcc:false`。
