# Xingtu TSP 最终状态（2026-11-23，40 轮收口）

> 本文是代码与测试事实清单，不是标准符合性、认证或生产就绪声明。  
> 真实终端、弱网、KMS/HSM、第三方一致性测试按要求未执行。

## 1. 验证结果

```text
lib                    255 passed
integration            32 passed
fmt                    clean
clippy                 clean
```

集群：

```text
8090  Follower
8091  Leader
8092  Follower
```

协议监听：

- JT/T 808 TLS：`18080/18081/18082`
- 1078 媒体 UDP：`19000`
- GB/T 32960 TCP/TLS/mTLS：`19900`
- HTTP API：`8090/8091/8092`

## 2. 标准目录

`src/standards/catalog.rs`，81 项：

```text
Done            66
Partial          8
Missing          0
NotApplicable    7
```

`Missing=0` 表示目录中不再有“完全未实现”的条目，**不等于**全部标准已实现。

## 3. 已完成（代码侧）

### JT/T 808-2019

- 帧、转义、BCC、BCD、分包
- 0x0001/0x8001、0x0002、0x0102
- 0x0104/0x0107/0x0108
- 0x0200 基础体 + 常用 TLV：
  - 0x01/0x02/0x03/0x04/0x05/0x06
  - 0x11/0x12/0x13
  - 0x25/0x2A/0x2B/0x30/0x31
  - 未知 TLV 保留 hex
- 表 24/25 状态位/报警位
- 0x0201、0x0205
- 0x0701、0x0702、0x0705
- 0x0800/0x0801/0x0802/0x0805
- 0x0901、0x8A00/0x0A00
- 0x8400/0x8401
- 0x8600–0x8608、0x0608
- 0x8700/0x0700、0x8702
- 0x8800–0x8805
- 0x8900/0x0900
- 修改单 0x0205、0x0702

### JT/T 809-2019

- 30 字节头、转义、CRC32、可配置加密
- 登录/注销/链路保持基础
- 0x1300/0x9300、0x1301/0x9301、0x1302、0x1303/0x9303
- 车辆监管：
  - 0x9501–0x9505
  - 0x1501–0x1505（含 0x1502 拍照 + GNSS 体）
- 静态信息：
  - 0x9601、0x1601、0x1602
  - 表 91 `KEY:=VALUE` 解析
- 报警：
  - 0x1402、0x1412
  - 0x9401/0x1401 督办
  - 0x9402 预警
  - 0x9403 实时交换
- 修改单表 34/36 驾驶员身份证/人脸匹配度/UID

### GB/T 32960.3-2025

- `$$`/`##`、BCC
- 2016/2025 车辆登入
- 平台登入/登出、心跳
- 0x0B 密钥交换
- 报警等级 + N5
- 0x07/0x08 电压/温度
- 0x30/0x31/0x32 电堆/超级电容
- 签名、激活、自定义
- RSA/AES/SM4/SM2/ECDSA
- 1h 监测状态机
- 密文帧：BCC 后解密
- HTTP 入库
- TCP 流分帧 + TCP/TLS/mTLS 监听
- VIN 白名单、设备注册表、管理 API、按 VIN 限流

### GB/T 19056-2021

- 表 A.2 记录块（55 76，16 字节对齐，XOR）
- 表 A.3 起始记录块
- 表 A.5 验证记录块
- SM3 + 16 字节 Salt
- 文件级摘要链
- 21H 行驶状态
- 22H 事故疑点
- 23H 超时驾驶
- 24H 驾驶人信息/照片
- 25H 日志
- 2012 AA75/557A legacy

### JT/T 1078-2016

- 30 字节媒体头/包体
- 0x9101/0x9102
- 0x9201/0x9202
- 0x9205/0x1205
- 0x9206/0x9207/0x1206
- 序列重排 + marker 组帧
- H.264/AAC 解码
- fMP4/MSE 回放

### 地方标准

- DB31/T 1409.1/.2/.3-2023 全部 **2026-07-08 废止**
- 未检索到正式替代标准
- 目录标记 `NotApplicable`

## 4. 仍然 Partial 的 8 项

### 协议字段类

- 808 0x0100 注册字段完整性
- 808 0x0200 少数 TLV 语义
- 808 0x0704 分片/批量边界
- 808 0x0802 多条记录
- 808 0x8103/0x8104/0x8107
- 808 0x8202/0x8203/0x8204
- 808 0x8300、0x8500
- 808 0x8700/0x0700 与 19056 命令映射
- 809 0x1200/0x9200、0x1300/0x9300
- 32960 0x02/0x03 全 section 业务语义
- 32960 0x08 校时

### 媒体类

- 1078 媒体包完整边界
- 1078 信令全子类型
- 1078 回放/转封装
- JT/T 1077 平台要求

### 平台/合规类

- JT/T 796 平台要求
- GB/T 35658 平台要求
- GB/T 22239 等保
- GB/T 41871 汽车数据安全
- GB/T 35273 个人信息安全
- GB/T 39786 密码应用
- GB/T 40855 电动汽车远程服务信息安全

这些属于“要求类”标准，代码只能覆盖部分技术控制，不能以测试替代评估。

## 5. 未做/不可声明

- 真实 808/809/1078/32960 终端联调
- 弱网、丢包、抖动、长稳测试
- KMS/HSM 生产密钥托管
- 证书吊销/轮换
- 第三方一致性测试
- 等保测评、密评、数据安全评估
- 生产容量与 SLA

## 6. 性能现状

本机 release 编解码基线：

- 32960 编 ~2.54M ops/s
- 32960 解 ~1.44M ops/s
- 809 编 ~0.49M ops/s（最坏转义）
- 809 解 ~0.40M ops/s

API：

- Viewer 车辆列表约 6–9ms
- Admin 受 PG fsync/Defender 影响，波动大

这些是开发机数据，不是生产承诺。

## 7. 结论

代码侧已完成规划中的主要协议族与关键链路：

```text
808 帧/位置/控制/多媒体/修改单
809 帧/监管/拍照/静态信息/报警/修改单
32960 2025 数据体/密码/密文/接入/入库/准入
19056 2021 容器/五类记录/摘要链
1078 包/信令/重组/解码/回放
```

未完成部分主要是：

- 真实设备与第三方验证
- 要求类标准的组织与合规工作
- 少量边缘字段/子类型
- 生产密钥、证书、容量与运维

因此：

> **可以声明“主要协议代码与本地测试已完成”，不能声明“标准符合、认证通过或生产就绪”。**

## 8. 代码侧收口（2026-11-23 最终轮）

目录：

```text
Done            66
Partial          8
Missing          0
NotApplicable    7
```

验证：

```text
lib                    255 passed
integration            32 passed
fmt                    clean
clippy -D warnings     clean
cluster                3 nodes healthy
```

剩余 8 项均为要求类/外部验证：

- 视频平台技术要求
- 另外两个平台技术要求
- 等保 2.0
- 汽车数据安全
- 个人信息安全
- 密码应用
- EV 远程服务信息安全

结论：

> 代码侧协议与主要平台链路已完成规划范围；无 Missing。
> 真实终端/上级平台/弱网/第三方测评未执行，不能声明标准符合、认证或生产就绪。

## 9. RustFS 录制/归档

已接入：

```text
1078 UDP -> MediaHub 实时播放
        -> 关键帧/时间/大小分段
        -> raw H.264/AAC 异步上传 RustFS
        -> /api/media/archive 列表/下载
```

配置：

```text
TSP_MEDIA_UDP_ADDR
TSP_RUSTFS_ARCHIVE
TSP_RUSTFS_ENDPOINT/BUCKET/REGION/ACCESS_KEY/SECRET_KEY
TSP_RUSTFS_PREFIX
TSP_RUSTFS_SEGMENT_SECS/BYTES
```

验证：

```text
archive_put_list_get_against_s3_mock
archive_playback_pairs_video_and_audio_keys
upload_retries_transient_s3_failures
render_includes_archive_counters
media_keys_are_prefixed_and_path_safe
prefix_and_segment_limits_have_safe_defaults
```

未验证：真实 RustFS 容器、真实 1078 终端、断点/限流/大文件/并发上传、归档 raw 的浏览器回放。

## 10. 时序后端选择（TDengine）

新增 `TSP_TIMESERIES_BACKEND`：

```text
postgres  小/中型，PostgreSQL/PostGIS
tdengine  大型，TDengine 查询 + PG 权威
dual      迁移期双写，查询优先 TDengine
```

- 配置/启动校验：`tdengine` 必须成功初始化，`dual` 失败降级 PG
- 健康检查输出 `timeseries_backend` 与 `tdengine`
- 轨迹查询：tsink（可选）→ TDengine → PostgreSQL
- 轨迹导出使用同一后端选择
- Compose profile：`--profile tdengine`
- K8s 可选清单：`deploy/k8s/tdengine.yaml`
- 文档：[TIMESERIES_BACKENDS.md](TIMESERIES_BACKENDS.md)

测试：

```text
config::tests::parses_time_series_backends
tdengine::tests::parses_timestamps_and_identifiers
tdengine::tests::rest_query_maps_rows_to_points
```

写入可靠性：

- 有界队列 + 3 次指数退避重试
- 重试耗尽写磁盘 spool，后台重放
- 指标：writes/failures/retries/dropped/queue_depth/spooled/replayed/pending

对账：

- `GET /api/integrations/timeseries/reconcile`
- 比较 PG/TDengine count/min/max
- 结果写入 `timeseries_reconciliations`
- `GET /api/integrations/timeseries/reconciliations` 查询历史
- 指标：checks/mismatches
- mismatch 可推送 JSON webhook（全局冷却）
- `GET /api/integrations/timeseries/spool` / `POST .../spool/replay`
- 历史支持 `vehicle_id` / `consistent` 过滤，默认保留 90 天
- webhook 支持 HMAC-SHA256 签名、重试、`generic`/`alertmanager` 负载

健康：

- `tdengine_ready` 来自 `SELECT SERVER_VERSION()`
- `tdengine` 模式下 `/api/ready` 依赖 TDengine

部署验证：

- Compose profile `tdengine`
- K8s `deploy/k8s/tdengine.yaml`
- `scripts/tdengine-smoke.ps1`（语法校验；真实容器未运行）

写入可靠性：

- 有界队列 + 3 次指数退避重试
- 重试耗尽写磁盘 spool，后台重放
- 指标：writes/failures/retries/dropped/queue_depth/spooled/replayed/pending

对账：

- `GET /api/integrations/timeseries/reconcile`
- 比较 PG/TDengine count/min/max
- 指标：checks/mismatches

健康：

- `tdengine_ready` 来自 `SELECT SERVER_VERSION()`
- `tdengine` 模式下 `/api/ready` 依赖 TDengine

部署验证：

- Compose profile `tdengine`
- K8s `deploy/k8s/tdengine.yaml`
- `scripts/tdengine-smoke.ps1`（语法校验；真实容器未运行）

真实 TDengine 容器未运行；REST 适配器已用 mock 验证。

