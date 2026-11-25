# Room / Live / Meeting

## Live API（2026-11-21）

| 接口 | 说明 |
|---|---|
| `/api/live/join?room=&user=` | 加入房间 |
| `/api/live/leave?room=&user=` | 离开 |
| `/api/live/gift?room=&user=&gift=&count=` | 礼物 |
| `/api/live/pk?room=&side=&points=` | PK |
| `/api/live/rooms` | 房间列表（含 title/closed/members） |
| `/api/live/room/create?room=&title=` | 创建 |
| `/api/live/room/close?room=` | 关闭 |
| `/api/live/seat/take?room=&user=&index=` | 上麦 |
| `/api/live/seat/leave?room=&user=` | 下麦 |
| `/api/live/seat/list?room=` | 麦位列表 |
| `/api/live/kick?room=&user=` | 踢人（移除成员/麦位/禁言） |
| `/api/live/mute?room=&user=&on=1` | 禁言/解除 |
| `/api/live/robots?room=&count=` | 机器人/虚拟观众计数 |
| `/api/live/status?room=` | members / pk / seats / muted / robots / gifts |
| `/api/live/chat/send?room=&user=&text=` | 发送弹幕/聊天（禁言/敏感词过滤） |
| `/api/live/chat/list?room=&limit=` | 聊天历史 |
| `/api/live/chat/clear?room=` | 清空聊天 |

麦位默认 8 个；同一用户换麦会自动释放旧麦位。

### 持久化

`Live::to_json` / `from_json` 现在包含：

```text
rooms / capacity / seats / muted / robots
```

测试：

```text
seat_tests::seats_mute_kick
persist_tests::save_and_load
tests/room_http.rs::live_seats_mute_kick
tests/room_http.rs::live_chat_moderation_http
```

## Meeting API

| 接口 | 说明 |
|---|---|
| `/api/meeting/join` / `leave` | 成员进出 |
| `/api/meeting/publish?on=1` | 发布状态 |
| `/api/meeting/rooms` | 房间列表 |
| `/api/meeting/status?room=` | members/publishers/muted/hands |
| `/api/meeting/mute?room=&user=&on=1` | 会议禁言 |
| `/api/meeting/hand?room=&user=&on=1` | 举手 |
| `/api/meeting/kick?room=&user=` | 移出会议 |
| `/api/meeting/stream?room=&stream=` | 绑定房间流 |
| `/api/meeting/record?room=&secs=` | 录制房间流到 MP4 |

## 聊天/审核

- 禁言用户不能发言；
- `badword`/`forbidden` 及 `M7S_LIVE_BANNED_WORDS` 过滤；
- 每房间保留最近 200 条。

## 仍开放

- 连麦混流（Mix）真正合成；
- 审核/敏感词/录制回放；
- 机器人行为脚本、虚拟观众真实推流；
- 多房间 PK 计分持久化。

## ¼���б�/�طţ�2026-11-22��

```text
GET /api/recordings                  FLV/MP4 �б�
GET /recordings/<file>               ֱ�ӻط� FLV/MP4
GET /api/meeting/recordings          MP4 �б�
GET /api/meeting/playback?file=<f>   MP4 �ط�
```

Ŀ¼��

```text
M7S_RECORD_DIR=data/record
M7S_MP4_DIR=data/mp4
```

��ȫ���ܾ� `..` / ·���ָ�������� `tests/recording_http.rs`��

## ���� HTTP��2026-11-22��

```text
GET /api/mix?input=lavfi:color=red:s=64x64:d=1,lavfi:color=blue:s=64x64:d=1&secs=1
```

- ���붺�ŷָ��֧�� `lavfi:` / �ļ� / RTSP URL��
- ��� `data/mix/mix-<ts>.mp4`��
- ���ԣ�`tests/room_http.rs::mix_http_generates_mp4`��

Room �Զ�ȡ��·���������ı����Դ����롣

## �����˽ű���2026-11-22��

```text
GET /api/live/bot?room=&user=bot1&text=hello,world&interval=1000
GET /api/live/bot/stop?room=
```

- �� `text` �����б�ѭ���������죻
- interval ��С 10ms��
- ÿ����һ�� bot���ظ�����滻�� bot��
- ���ԣ�`chat_tests`/`bot_tests::bot_posts_and_stops`��

�߼���������Ϊ������/����/����Դ���չ��

## Room �Զ�������2026-11-22��

```text
GET /api/live/mix?room=<room>&other=<room2>&secs=2
```

- �� meeting ������ӳ��ȡ��·��`lavfi:`/�ļ�/RTSP ���ɣ���
- ��� `data/mix/room-<ts>.mp4`��
- ���ԣ�`tests/room_http.rs::room_auto_mix_uses_room_streams`��
