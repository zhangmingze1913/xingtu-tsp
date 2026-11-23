# 星途 TSP 集群架构设计

> 目标：把单机演示版演进为可横向扩展、可多可用区容灾的车联网 TSP 集群。  
> 对标：JT/T 808-2019、JT/T 809-2019、GB/T 32960 等部标体系，以及头部车队平台普遍采用的“接入网关集群 + 消息总线 + 领域服务 + 热/温/冷分层存储 + 实时通道”架构。

## 1. 架构目标

| 指标 | 目标 |
| --- | --- |
| 可用性 | 99.95%（单区域多 AZ），关键链路 99.99% |
| 接入规模 | 100 万在线终端，峰值 10 万 msg/s，可水平扩展到 500 万 |
| 位置时效 | P95 < 2s 入库，P95 < 3s 推送控制台 |
| 车控时效 | P95 < 2s 下发，5s 内可观测回执 |
| RPO / RTO | RPO = 0（同步副本）；RTO < 5 分钟 |
| 数据保留 | 热 7 天、温 13 个月、冷 3 年（对象存储/归档） |
| 多租户 | 租户级数据隔离、配额、独立加密密钥 |

## 2. 业界权威参考模型

车联网 TSP 的权威架构通常分成七层，本项目按此对标：

```text
┌─────────────────────────────────────────────────────────────┐
│ 7. 开放与生态   OpenAPI / 车队 SaaS / 保险 UBI / 充电 / 维保  │
├─────────────────────────────────────────────────────────────┤
│ 6. 运营与安全   多租户 / RBAC / 审计 / 等保 / 隐私合规         │
├─────────────────────────────────────────────────────────────┤
│ 5. 实时通道     WebSocket / SSE / 推送网关 / 规则通知          │
├─────────────────────────────────────────────────────────────┤
│ 4. 领域服务     车辆 / 轨迹 / 行程 / 围栏 / 告警 / 车控 / OTA   │
├─────────────────────────────────────────────────────────────┤
│ 3. 消息与计算   Kafka / Flink / 规则引擎 / 时序计算            │
├─────────────────────────────────────────────────────────────┤
│ 2. 接入网关     JT/T 808 / GB/T 32960 / MQTT / HTTP 集群       │
├─────────────────────────────────────────────────────────────┤
│ 1. 终端与边缘   T-Box / 车机 / 充电桩 / 路侧设备 / 边缘缓存      │
└─────────────────────────────────────────────────────────────┘
```

协议边界：

- **JT/T 808-2019**：营运车辆终端接入、注册鉴权、位置、报警、车控、OTA。
- **JT/T 809-2019**：部/省/企业平台间数据交换（监管平台对接）。
- **GB/T 32960**：新能源汽车远程服务与管理系统（三电、充电、故障码）。
- **MQTT / HTTP**：车机、充电桩、第三方生态接入；TLS/mTLS。
- **JT/T 808** 的 TCP 长连接必须由**接入网关集群**承接，不能把连接绑在业务 API 上。

## 3. 集群逻辑架构

```text
                   ┌──────────── L4 LB / NLB ─────────────┐
                   │                                      │
        ┌──────────▼──────────┐              ┌────────────▼──────────┐
        │ JT808 Gateway ×N    │              │ Ingress / API LB       │
        │ 无共享状态           │              │ HTTPS + WebSocket     │
        │ 会话注册到 Redis     │              └────────────┬──────────┘
        └──────────┬──────────┘                           │
                   │ 位置/事件                              │
                   ▼                                       ▼
        ┌─────────────────────┐                 ┌─────────────────────┐
        │ Kafka / Redis Stream│◀───────────────▶│ API Service ×N       │
        │ 按 VIN 分区、可回放   │                 │ 无状态、JWT、RBAC     │
        └──────────┬──────────┘                 └──────────┬──────────┘
                   │                                       │
       ┌───────────▼───────────┐              ┌────────────▼──────────┐
       │ 流计算 / 规则引擎 ×N   │              │ WebSocket Gateway ×N  │
       │ 行程、围栏、告警、去抖  │              │ Redis Pub/Sub 扇出     │
       └───────────┬───────────┘              └────────────┬──────────┘
                   │                                       │
        ┌──────────▼───────────────────────────────────────▼─────────┐
        │ PostgreSQL / TimescaleDB（业务 + 热轨迹）                  │
        │ ClickHouse / HBase（温冷轨迹、分析）；Redis（在线态/锁）     │
        │ 对象存储（原始报文、归档、OTA 包）                          │
        └────────────────────────────────────────────────────────────┘
```

关键原则：

1. **接入与业务分离**：Gateway 只做协议、鉴权、连接、限流、转发；业务规则不进入网关。
2. **无状态服务**：API、WebSocket、规则 worker 不保存会话；状态进 Redis/PG/Kafka。
3. **分区有序**：Kafka 以 VIN 为 key，保证同一车的位置、报警、指令顺序。
4. **至少一次 + 幂等**：位置消息带 `vehicle_id + ts + seq` 幂等键；写库使用唯一约束/去重表。
5. **热温冷分层**：7 天热轨迹在 Timescale/分区表，历史轨迹进 ClickHouse/对象存储。
6. **控制面与数据面分离**：节点注册、选主、配置、灰度通过控制面；车控指令走独立高优先级通道。

## 4. 接入层集群

### 4.1 连接分布

```text
终端 ──TCP──▶ L4 负载均衡（源地址哈希/最小连接）
                 ├── jt808-gw-0  (连接 1..N)
                 ├── jt808-gw-1
                 └── jt808-gw-2
```

- 网关 Pod 无状态（除 TCP socket）；不依赖本地磁盘。
- 连接断开后终端重连可落到任意网关，因此**会话不能只存本地内存**。
- Redis 记录 `gateway:session:<phone> -> {node_id, connected_at, remote_ip, protocol}`，TTL 90s，心跳续期；用于车控下行寻址与运维排障。
- 网关节点注册 `gateway:node:<node_id>`，TTL 30s；控制面可感知在线网关数。
- 单连接读缓冲、最大帧长、单终端速率限制；异常帧计数进 Prometheus。

### 4.2 下行指令路由

```text
命令服务 ──▶ 查 Redis session ──▶ 发布到 gateway:<node_id> 频道
                                         │
                                  目标网关订阅并写回 TCP 连接
```

若终端已离线，返回 `409/503`，进入指令补偿队列；命令状态机写入 PostgreSQL，保证多 API 实例看到同一状态。

### 4.3 协议扩展

- JT/T 808 分包、0x0704 批量位置、0x8300 文本、0x8500 车控、0x8103 参数、OTA。
- GB/T 32960 独立 adapter，统一转成内部 `LocationSample`/`VehicleSignal`。
- MQTT 接入使用 EMQX/HiveMQ 集群，桥接到 Kafka。
- 所有 adapter 只依赖领域事件，不直接写业务表。

## 5. 消息与计算

### 5.1 Topic 设计

| Topic | Key | 保留 | 用途 |
| --- | --- | --- | --- |
| `tsp.location.raw` | VIN | 7d | 原始位置/报警帧 |
| `tsp.location.normalized` | VIN | 7d | 标准化位置事件 |
| `tsp.trip.events` | VIN | 30d | 行程开始/结束 |
| `tsp.alert.events` | VIN | 30d | 规则告警 |
| `tsp.command.requests` | VIN | 7d | 车控下行 |
| `tsp.command.receipts` | VIN | 7d | 终端回执 |
| `tsp.device.lifecycle` | device_id | 30d | 注册/离线/OTA |

分区数按峰值吞吐设计：单分区 2–5 MB/s，100 万车 30s 上报约 3.3 万 msg/s，建议 64–128 分区，预留 3 倍峰值。

### 5.2 规则计算

- 行程切分、围栏、超速、低电量、急刹等规则在 Flink/独立 worker 中执行，不阻塞接入。
- 规则状态（围栏 inside/outside、告警去抖）存 Redis/PG，按 VIN 分区，避免跨实例竞争。
- 使用事件时间 + watermark 处理乱序；迟到数据可回放修正。
- 告警写入 `alerts` 并发布 `tsp.alert.events`，通知服务消费后发短信/企微/电话。

## 6. 数据分层

| 层 | 存储 | 内容 | 保留 |
| --- | --- | --- | --- |
| 在线态 | Redis Cluster | 车辆最新状态、终端在线、会话、围栏状态、分布式锁 | 秒级 TTL |
| 业务库 | PostgreSQL（主从/多 AZ） / MySQL 8 | 车辆、终端、用户、围栏、告警、指令、租户 | 永久 |
| 事件日志 | Apache Iggy / Kafka | 可回放的跨实例领域事件 | 7–30 天 |
| 热轨迹 | TimescaleDB / PG 分区表 / tsink / TDengine | 7 天原始轨迹 | 7 天 |
| 温轨迹 | ClickHouse | 13 个月轨迹、行程、聚合指标 | 13 个月 |
| 冷数据 | 对象存储 + Parquet | 原始报文、3 年轨迹归档 | 3 年 |
| 搜索 | OpenSearch | 车牌/VIN/告警全文检索 | 可重建 |
| 缓存 | Redis | 字典、权限、看板聚合 | 分钟级 |

写入路径：

```text
Gateway → Kafka → 标准化 → 批量写 Timescale（1–5s 批次）
                          → 更新 Redis 最新状态
                          → 规则计算 → 告警
```

不要在每个位置点同步写多张业务表；批量、异步、可回放是规模化的前提。

## 7. 实时通道

- WebSocket Gateway 集群独立于 API，可单独扩缩容。
- 每个节点订阅 Redis Pub/Sub 或 Kafka 的广播 topic，按租户/车辆权限过滤后推送。
- 本项目也支持 **Apache Iggy** 作为可选持久事件日志：`TSP_IGGY_URL` 开启后，事件通过 Iggy HTTP 流写入，每个 API/网关节点使用独立 consumer 消费同一分区；Redis 继续负责 Leader 租约和网关会话。
- 不需要 sticky session；断线重连后通过 `resync` + REST 补齐。
- 大屏/车队场景使用“订阅关系表”：`user/tenant -> vehicle set`，Redis Set 维护。
- 推送限流：单连接 QPS、消息合并（100ms 窗口）、慢客户端丢弃并触发 resync。

## 8. 调度与一致性

| 任务 | 方式 |
| --- | --- |
| 离线检测 | Leader 选举（Redis/etcd/K8s Lease）+ 分片扫描 |
| 围栏/规则 | 按 VIN 分区到 worker，Kafka consumer group |
| 指令超时补偿 | PG `SKIP LOCKED` 队列 + 定时扫描 |
| 报表聚合 | 定时任务 + 分布式锁 |
| 数据清理 | 分区 drop / TTL，不做大事务 delete |

一致性策略：

- 位置写入幂等：`UNIQUE(vehicle_id, ts, seq)` 或 Redis 去重窗口。
- 行程/围栏使用 **事件版本号** 防止乱序覆盖。
- 指令状态机：`pending → sent → success/failed`，每次迁移带 `version` 乐观锁。
- 跨服务最终一致，关键车控使用 saga/补偿。

## 9. 多租户与安全

- 所有业务表带 `tenant_id`；JWT 携带 `tenant_id/org_ids`；SQL 强制租户过滤（RLS 或 repository 层）。
- 租户级 KMS 密钥、数据导出审计、配额（车辆数/消息数/API QPS）。
- 等保 2.0 三级：边界防护、入侵检测、日志留存 6 个月、双因素、密钥托管。
- 个人信息最小化：驾驶员手机号脱敏、位置数据访问审计、离职回收。
- 车控安全：二次确认、双人复核（高危指令）、指令签名、防重放 nonce、终端白名单。

## 10. 高可用与容灾

### 10.1 单区域多 AZ

- API/Gateway/WS：≥3 副本，跨 3 AZ 打散；PDB 保证滚动升级至少 2 个可用。
- PostgreSQL：1 主 2 同步/异步副本，Patroni/CloudNativePG 自动 failover。
- Kafka：3 broker，RF=3，min.insync.replicas=2。
- Redis：Cluster/Sentinel，3 主 3 从。
- 对象存储：多 AZ 冗余。

### 10.2 跨区域灾备

- 业务库：跨region 异步复制；RPO 分钟级。
- 轨迹：Kafka 跨集群复制（MirrorMaker 2）+ 对象存储跨区复制。
- DNS/GSLB 切换；车控在灾备区默认只读，避免双写。
- 每季度做一次切换演练，记录 RTO。

## 11. 可观测性与 SRE

指标：

- 接入：连接数、鉴权失败率、帧错误率、上报延迟、单终端速率。
- 业务：位置入库 TPS、Kafka lag、行程生成量、告警量、指令成功率/P95。
- 数据：PG 复制延迟、慢查询、Redis 命中率、磁盘水位。
- 体验：WebSocket 在线数、推送延迟、resync 率。

SLI/SLO：

```text
位置入库可用性 = 成功入库 / 总上报 ≥ 99.95%
位置时效 P95 < 2s
车控成功率 ≥ 99.5%（在线车辆）
API 可用性 ≥ 99.95%
```

链路追踪：OpenTelemetry，trace_id 从网关贯穿 Kafka 到入库；日志结构化 JSON；告警接 PagerDuty/企微。

## 12. 容量模型（100 万车）

| 指标 | 估算 |
| --- | --- |
| 在线终端 | 1,000,000 |
| 上报周期 | 30s（行驶中可 10s） |
| 平均消息 | 3.3 万/s；峰值 10 万/s |
| 单条位置 | ~250B（含协议/索引） |
| 日增原始轨迹 | ~700GB/天（含副本前） |
| 热存储 7 天 | ~5TB（压缩后 1–2TB） |
| Kafka | 64–128 分区，3 副本，7 天保留 |
| PG | 按租户分库/分表；单实例 1–2TB 内 |
| Redis | 最新状态 ~2GB，会话/锁 ~1GB |

网关容量按“单 Pod 2–5 万长连接”估算，100 万连接需 20–50 个网关 Pod；按连接数 HPA（自定义指标），不要只看 CPU。

## 13. 部署拓扑（Kubernetes）

```text
namespace: xingtu-tsp
├── Deployment  api         3+ 副本  (REST + WebSocket)
├── Deployment  jt808-gw    3+ 副本  (TCP 18080, L4 LB)
├── Deployment  worker      2+ 副本  (Flink/规则/补偿)
├── StatefulSet postgres    3 副本  (或 CloudNativePG)
├── StatefulSet redis       3 主 3 从 (或托管 Redis)
├── StatefulSet kafka       3 broker (或托管 Kafka)
├── Ingress     tsp-web     HTTPS + WS
└── HPA/PDB/NetworkPolicy/ServiceMonitor
```

节点池：

- `gateway` 池：高网络带宽、大连接数、关闭 CPU 超卖。
- `api` 池：通用计算。
- `data` 池：本地 SSD，PG/Kafka/Redis 反亲和。
- `observability` 池：Prometheus/Loki/Tempo。

部署清单见 [`deploy/k8s`](../deploy/k8s) 与 [`deploy/README.md`](../deploy/README.md)。

## 14. 当前实现与目标架构的映射

| 能力 | 当前代码 | 集群目标 |
| --- | --- | --- |
| HTTP API | Axum 单体，JWT/RBAC，可多副本 | 无状态 API Deployment ×N |
| JT/T 808 | TCP 网关 + Redis 会话注册 | Gateway ×N + L4 LB + Redis 会话 |
| 实时推送 | 进程内 broadcast + Redis Pub/Sub | Kafka/Redis 扇出 |
| 调度 | Redis 租约选主 | Leader 选举 + 分片 worker |
| 业务库 | SQLite（本地）/ PostgreSQL / MySQL | PostgreSQL 主从 / 多 AZ 或 MySQL 主从 |
| 轨迹 | SQLite telemetry / PostgreSQL / MySQL 表 | TimescaleDB / tsink / TDengine + ClickHouse 分层 |
| 消息 | 同步入库 + Redis/Iggy 事件 + Walrus 归档 | Kafka 分区、可回放 |
| 租户 | 单租户 | `tenant_id` + RLS |
| 可观测 | tracing + Prometheus 指标 | Prometheus + OTel + SLO |

本仓库已提供的集群化基础：

1. **SQLite/PostgreSQL/MySQL 三后端**：`TSP_DATABASE` 接受文件路径、`postgres://` 或 `mysql://` URL。
2. **无状态 API**：JWT，无本地会话；可直接多副本。
3. **事件总线**：Redis Pub/Sub（默认）或 Apache Iggy HTTP 持久事件日志（`TSP_IGGY_URL`）。
4. **Leader 选举**：模拟器/巡检任务只在一个节点执行；gateway 角色不参与抢锁。
5. **网关会话注册表**：终端号 → 网关节点，用于下行寻址。
6. **并发初始化保护**：PostgreSQL schema 使用事务级 advisory lock，demo seed 使用会话级 advisory lock；MySQL 使用 `GET_LOCK` 串行化首次种子。多个副本同时启动不会重复种子。
7. **可选时序库**：tsink HTTP（Prometheus text import + PromQL）或 TDengine REST SQL 用于轨迹热存储；查询顺序为 tsink → TDengine → 关系库，逐级回退。
8. **可选 Walrus 归档**：Walrus 长度前缀 TCP 的 `REGISTER/PUT` 将领域事件写入持久日志；Walrus topic 是共享 cursor，不承担多节点 WebSocket 广播。
9. **可选 OpenRaft 控制面**：`TSP_RAFT_ENABLED=true` 时，调度器 Leader 与集群成员元数据由 OpenRaft（Raft 选举/日志）管理；Raft RPC 使用 JSON over HTTP，节点通过 `TSP_RAFT_PEERS` 静态配置。Redis 继续负责网关会话和事件总线，PG/MySQL 继续承载业务数据。
10. **Kubernetes 清单**：API/Gateway/Ingress/HPA/PDB/NetworkPolicy；可选 Iggy StatefulSet。
11. **健康检查与指标**：`/api/health`、`/api/metrics`（Prometheus）、`/api/cluster/status`。

当前 Redis 适配器面向**单端点 Redis**（含 Sentinel/Proxy 提供的统一入口，或托管 Redis 的 primary endpoint）。若直接使用 Redis Cluster 多分片，需要把 `src/cluster.rs` 的 `Client` 替换为 `redis::cluster_async` 或通过集群代理接入；Pub/Sub 在 Redis Cluster 中本身是节点广播语义，生产更推荐用 Kafka 做跨实例事件总线。

Iggy 适配器使用 **HTTP REST**（默认 3000）而非原生 SDK，以避免把 Iggy 的重型 TLS/QUIC 依赖强加给每个部署；每个节点独立消费同一分区。当前已在本地用 Iggy HTTP 契约 mock 验证登录/建流/建 topic/生产/消费；生产接入真实 Iggy 0.10 时请以实际 `/streams` 响应字段为准做一次联调。

tsink 适配器使用 **Prometheus 兼容 HTTP**（`/api/v1/import/prometheus`、`/api/v1/query_range`），以 `tsp_lat/tsp_lon/tsp_speed/...` 多指标按 `vehicle_id` 查询后在服务内合并为轨迹点；已用真实 `tsink-server 0.10.2` 验证该路径。TDengine 适配器使用 taosAdapter REST SQL，自动建库/超级表，按车辆子表写入并用 tag 过滤查询；当前用 REST 契约 mock 验证。Walrus 使用长度前缀 TCP 协议，作为持久事件归档/下游流，不参与实时广播；协议已从上游 `client.rs` 核实为双向 4 字节小端长度前缀，上游服务端在 Windows 上受存储后端限制，实际运行需 Linux。

### OpenRaft 控制面

`src/raft.rs` 使用 OpenRaft 0.10 + in-memory memstore：

- 静态成员：`TSP_RAFT_PEERS=1@host:7001,2@host:7002,3@host:7003`
- RPC：`POST /raft/{append,vote,pre-vote,snapshot,transfer-leader}`，JSON over HTTP
- 选主结果写入现有 `leader` 原子标志，模拟器/巡检只在 Raft Leader 执行
- `/api/cluster/status` 返回 `raft_enabled/raft_node_id/raft_leader/raft_term/raft_state/raft_last_log`
- Raft 状态机是**控制面**，不复制业务表；memstore 重启后为空，生产可替换为持久化存储实现
- 已本地验证 3 节点选举与 kill Leader 后的重新选主

仍需补齐的生产级能力：

- Kafka/Flink 流式管道与轨迹分层（当前为同步入库）。
- JT/T 808 分包、批量位置、0x8300/0x8500 下行与 OTA。
- 多租户、RLS、租户级配额。
- PG 分区/分库、轨迹归档、跨区域容灾。
- 指令下行的 Redis 频道实际投递（当前命令为仿真回执）。
- 全链路压测与故障演练。

## 15. 演进路线

| 阶段 | 目标 | 关键动作 |
| --- | --- | --- |
| P0（当前仓库） | 可集群编排的垂直切片 | PG/Redis、多副本 API/Gateway、K8s |
| P1 | 单区域生产可用 | Kafka、Flink、Timescale、多租户、808 完整协议、压测 |
| P2 | 高可用与容灾 | 多 AZ、PG 自动 failover、跨区灾备、混沌演练 |
| P3 | 规模化与生态 | 分库分表、ClickHouse、OpenAPI、保险/充电/维保生态 |

## 16. 集群验收清单

- [ ] 2 个 API 副本同时在线，任一 Pod 被杀，控制台与 WebSocket 在 5s 内恢复。
- [ ] 2 个 Gateway 副本，终端重连到不同节点后位置仍连续入库。
- [ ] Redis 断开后集群降级策略明确（事件丢失可 resync，不丢业务库数据）。
- [ ] PG 主库切换后 API 自动重连，RTO < 5 分钟。
- [ ] Leader 切换后模拟器/巡检任务不重复执行。
- [ ] HPA 按连接数/CPU 扩容；PDB 保证升级期间可用。
- [ ] `/api/health` 在 DB/Redis 故障时返回 degraded，LB 摘除。
- [ ] 压测达到目标 TPS 与 P95，无消息积压或内存泄漏。
- [ ] 审计日志、指标、追踪、告警闭环可查。
