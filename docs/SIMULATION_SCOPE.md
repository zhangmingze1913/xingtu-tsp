# 合成一致性语料说明

> 日期：2026-11-19  
> 目的：在缺少授权标准全文/真实终端环境时，用**明确标注的合成样本**驱动协议实现和回归测试。  
> 红线：合成样本不是标准认证依据，也不能替代真实终端/监管平台联调。

## 1. 合成样本

| 文件 | 覆盖 | 来源 |
| --- | --- | --- |
| `tests/fixtures/standards/jt808_0100_2019.hex` | 0x0100 76 字节候选布局 + GBK 车牌 | 公开字段表推导 |
| `tests/fixtures/standards/jt808_0107_2019.hex` | 0x0107 候选属性布局 | 公开字段表推导 |
| `tests/fixtures/standards/jt808_0200_mandatory.hex` | 0x0200 + 0x01/02/03/04/11/12/13/25/30/31 | 公开附加项表推导 |
| `tests/fixtures/standards/jt808_8203.hex` | 0x8203 候选 2+4 字节 body | 公开字段表推导 |
| `tests/fixtures/standards/jt809_login.hex` | 809 登录帧 + CRC32 | 代码crc32计算 |
| `tests/fixtures/standards/gb32960_realtime.hex` | 32960 实时帧头 + VIN | 公开字段表推导 |

测试：

```text
synthetic_0100_0107_0200_8203_fixtures
static_frame_fixtures_decode
```

## 2. Phase 1 范围决策

| 项 | 决策 |
| --- | --- |
| 0x0100/0x0107 字段宽度 | 按 2019 公开表采用 11/30/30 候选布局，保留旧 37 字节兼容 |
| 0x0200 必选附加项 | 以 0x01/02/03/04/11/12/13/25/30/31 作为本项目合成必选集 |
| 0x8203 | 采用 2 字节流水号 + 4 字节类型位图候选布局 |
| 协议层加密 | **Phase 1 不实现**；传输层使用 TLS/mTLS，协议体加密待项目明确后单独立项 |
| 真实终端互通 | 未验证；合成样本只证明解析/编码与回归，不证明互通 |

## 3. 验收口径

可以说：

- 合成样本下，0x0100/0x0107/0x0200/0x8203 的编解码和回归测试通过；
- 0x0704 幂等、参数版本、超时重发等工程能力通过测试。

不能说：

- 符合 JT/T 808-2019；
- 通过终端互通/监管平台联调；
- 通过第三方检测；
- 协议层加密已实现。

## 4. 升级为真实验收的条件

1. 获得授权标准全文，核对字段宽度；
2. 用真实终端抓包替换/补充 fixture；
3. 至少 2 家终端/平台互通；
4. 明确协议层加密范围并实现/测试；
5. 第三方检测或监管平台联调报告。

## 5. Phase 2 跳过项（项目决定）

按项目决定，Phase 2 代码侧按模拟联调收口，以下两项**显式跳过**：

- 809 协议体加密/压缩：代码继续显式拒绝 `encrypt != 0` / `compress != 0`；
- 真实监管平台联调：只保留本地 TCP/TLS 模拟平台测试。

这不等于 JT/T 809 符合性，只表示当前交付范围。

## 6. Phase 3 命令字修正

核对公开标准后发现原实现命令字错误，已修正：

| 命令 | 正确值 |
| --- | --- |
| 车辆登入 | 0x01 |
| 实时信息上报 | 0x02 |
| 补发信息上报 | 0x03 |
| 车辆登出 | 0x04 |
| 平台登入 | 0x05 |
| 平台登出 | 0x06 |
| 心跳 | 0x07 |
| 终端校时 | 0x08 |

实时帧 fixture 已改为 0x02；0x04 登出候选体已实现。

0x01 车辆登入原先误用了 JT809 的 username/password/57 字节布局，已改为：时间 BCD[6] + 登入流水号 WORD + ICCID[20] + 储能子系统数/编码长度/编码。

## 7. Phase 3 模拟范围收口

Phase 3 代码侧按“模拟一致性套件”收口：

- 覆盖 0x01/0x02/0x03/0x04/0x05/0x06/0x07 的结构与已实现字段；
- 测试文件 `tests/gb32960_sim.rs`；
- 不覆盖 AES/SM4/RSA 解密，加密帧显式拒绝；
- 不覆盖 GB/T 32960.4 真实一致性测试。

因此 Phase 3 不能声明标准符合性，也不能作为监管验收证据。

## 8. Phase 4 起步：JT/T 1078

仅实现 30 字节媒体报文头 + 数据体：

- 帧头标识 0x30 0x31 0x63 0x64
- RTP 标志、序号、SIM、通道、数据类型
- 64 位时间戳、I 帧/帧间隔、数据体长度

未实现：音视频编解码、分包重组、0x9101/0x9102 等信令、真实设备联调。

## 9. Phase 4 代码侧边界

JT/T 1078 已完成媒体报文、重排/重组、H.264/AAC 解析、H.264 Annex-B 重组与 SPS/PPS/IDR 检测、`media::playback` 播放管线与 `media::sync` 音视频漂移判定、`media::remux` H.264/AAC→fMP4（MSE 可消费）、`/api/media/sample.mp4`、`/media` MSE 演示页、`media::hub` 会话缓存、`media::udp` 1078 UDP 监听与 `/api/media/live/{sim}/{channel}` 实时输出（需 JWT 鉴权）、`media::ffmpeg` 外部 FFmpeg H.264→RGB24 与 AAC→PCM16 解码适配器（`TSP_FFMPEG`）、协议解析器确定性随机输入 panic 安全测试、信令与资源列表候选体、模拟端到端链路。

JT/T 1077 经核对是**视频平台技术要求**，不是通信协议编解码，不能伪造“1077 协议实现”；平台功能/性能要求需按运维与验收清单落实。

## 10. Phase 5 PII 脱敏（代码侧）

已实现手机号、VIN、身份证、邮箱、车牌、姓名掩码，`classify_key/should_mask` 数据分类，以及递归 JSON 脱敏 `redact_json`。

审计列表与 CSV 导出已接入用户名掩码、detail JSON 递归脱敏和自由文本手机号掩码；已增加 admin/operator/viewer 字段级访问策略、按角色脱敏、Salted SHA-256 伪匿名化，并接入车辆、终端、告警、行程查询、车辆/终端/告警/行程/轨迹 CSV 导出，stdout 日志行脱敏 Writer，AES-256-GCM 字段加密工具与密钥环（`TSP_PII_KEY` / `TSP_PII_KEY_FILE` / `TSP_PII_PREVIOUS_KEYS`，64 位十六进制），只存密文的 `pii_vault` 表与 Db 读写接口，面向 HSM/KMS 的 `KeyProvider` 抽象（本地实现为 `local-keyring`），以及终端 `auth_code`、车辆 `driver_phone`/`driver_name` 的金库落库/解密读取（设置 `TSP_PII_KEY` 时）（viewer 掩码 VIN/车牌/驾驶员/电话/IMEI/ICCID/告警消息）；未接入所有日志/API 调用点，未做加密存储和第三方合规评估。

## 11. Phase 5 代码侧收口

Phase 5 代码侧已完成：脱敏、字段级策略、日志/导出审计、AES-GCM、密钥环、PII 金库、密钥审计、留存清理、KMS HTTP 适配、合规控制快照。

仍为外部/未完成：

- 真实云 KMS/HSM 账号与证书联调
- 第三方合规评估/认证
- 真实设备与监管平台测试
- 生产密钥托管与轮换演练

因此 Phase 5 只能标记为“代码侧完成”，不能标记为合规或认证完成。

## 12. 发布硬化起步

新增 `/api/ready`：数据库可用即返回 200；集群 leader 作为状态字段报告，不阻断 Follower 就绪。用于容器/负载均衡就绪探针。真实发布演练仍属外部。

## 13. 备份与发布预检

- `scripts/backup-postgres.ps1` 支持 plain / custom / schema-only，生成 SHA-256 manifest。
- `scripts/restore-postgres.ps1` 校验 manifest，支持 psql 或 pg_restore。
- `scripts/release-preflight.ps1` 检查磁盘、PostgreSQL、Redis、health、ready。

已实际生成 custom 全量备份（约 94MB，107 个 TOC 项，SHA-256 校验通过）。并在 `F:\pgdata-restore` 独立实例（端口 55433）完成恢复演练：vehicles=1,110,018、users=6、schema_migrations=18、telemetry=1,373,774，恢复后临时实例已清理。

## 14. JT/T 1078 播放阶段收口

代码侧链路已完成并端到端验证：

```text
UDP 1078 包
  -> Jt1078Packet::decode
  -> MediaHub（按 SIM/channel）
  -> PlaybackSession 帧重组 / 音视频漂移
  -> FFmpeg H.264/AAC 解码
  -> FFmpeg remux fMP4
  -> /api/media/live/{sim}/{channel}/stream
  -> 浏览器 MSE 连续播放
```

已用内置 H.264/AAC 样本通过 UDP 注入验证：
- `/api/media/sessions` 返回会话
- 快照接口返回 ftyp
- stream 接口返回 200、chunked fMP4、ftyp

仍未完成（均需外部条件）：
- 真实 JT/T 1078 终端推流
- 真实终端音视频编码参数兼容性
- 生产级低延迟/丢包/乱序长时间验证
- 真实 KMS/HSM、第三方合规评估
