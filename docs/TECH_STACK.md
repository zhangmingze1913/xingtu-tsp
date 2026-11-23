# 技术组件选型分析（2026-11 复审）

## 0.1 本轮加固状态（2026-11）

- JWT：RS256 + JWKS + 15 分钟 access + 7 天 refresh（轮换/吊销）。
- Redis：支持 `rediss://` + AUTH；生产默认拒绝明文 `redis://`。
- JT808：支持 TLS 证书监听；生产默认拒绝明文（需显式 `TSP_JT808_ALLOW_PLAINTEXT=true`）。
- Access token：logout 按 `jti` 写 Redis 吊销，跨节点立即生效；剩余 JWKS 多 key 轮换、终端证书/白名单。
- 详见 [SECURITY.md](SECURITY.md)。

## 0. 结论摘要

| 结论 | 说明 |
| --- | --- |
| 核心栈适合 10K–100K 车辆在线 | Rust/Tokio/Axum + PostgreSQL/PostGIS + Redis 的组合成熟、延迟低，当前基准 10K/100K 交互接口在数十毫秒级 |
| 1M 在线需要架构升级 | 单节点 debug 基准中 dashboard 冷启动 3.62s、车辆列表 5 并发 RPS 4.6；需要预聚合、keyset 分页、多 API/读写分离 |
| 可选组件过多 | Redis Pub/Sub、Iggy、Walrus 三套事件通道；PostGIS、tsink、TDengine 三套轨迹存储。默认部署只应保留一条主链路 |
| SQLx `AnyPool` 是最大技术债之一 | 换来 SQLite/MySQL/PG 可移植，但失去编译期 SQL 校验和部分类型能力；生产建议 PG 专用 `PgPool` |
| OpenRaft 只适合控制面，不适合业务数据 | 当前日志/状态机是文件 JSON；生产应改用 etcd/K8s Lease，或替换为可靠 Raft 存储 |
| 安全基线需补齐 | JWT HS256 + 12h、默认开发密钥、JT808 明文 TCP、Redis 无 TLS/AUTH 假设，均需生产加固 |
| 标准合规不能外推 | JT/T 808 是子集；JT/T 809、GB/T 32960 是帧/登录体骨架，不构成认证或全量合规 |

## 1. 审计范围与版本

版本来自 `Cargo.toml` / `Cargo.lock`，不是“最新版”宣传。

| 组件 | 版本 | 当前角色 | 使用状态 |
| --- | --- | --- | --- |
| Rust | 2021 edition | 服务端语言 | 主栈 |
| Tokio | 1.54.1 | 异步运行时 | 主栈 |
| Axum | 0.8.9 | HTTP/WebSocket 框架 | 主栈 |
| Tower-HTTP | 0.6.11 | CORS/trace/static/compression | 主栈 |
| SQLx | 0.8.6（any/sqlite/postgres/mysql） | 数据访问/迁移 | 主栈 |
| PostgreSQL | 16.4（部署环境） | 主数据库 | 主栈 |
| PostGIS | 3.7.0（部署环境） | 轨迹/位置空间列与查询 | 可选，自动检测 |
| Redis | 0.32.7 | 会话/节点/leader lease/PubSub | 集群主栈 |
| OpenRaft | 0.10.0 | 控制面 leader/元数据 | 可选 |
| Apache Iggy | HTTP adapter（无 Rust SDK） | 可选持久事件日志 | 可选 |
| Walrus | 自定义 TCP adapter | 可选下游归档日志 | 可选，不建议默认 |
| tsink | HTTP adapter | 可选 Prometheus 风格轨迹存储 | 可选 |
| TDengine | REST SQL adapter | 可选时序轨迹库 | 可选 |
| jsonwebtoken | 9.3.1 | JWT 签发/校验 | 主栈 |
| Argon2 | 0.5.3 | 密码哈希 | 主栈 |
| Reqwest | 0.12.28（native-tls） | Iggy/TDengine/tsink/Raft HTTP | 主栈/可选适配 |
| `openraft-legacy` | 0.10 | 直接依赖但代码未见使用 | 待清理 |

依赖来源：

- [Axum](https://github.com/tokio-rs/axum)
- [SQLx](https://github.com/launchbadge/sqlx)
- [PostGIS](https://postgis.net/)
- [Redis client](https://github.com/redis-rs/redis-rs)
- [OpenRaft](https://github.com/databendlabs/openraft)
- [Apache Iggy](https://github.com/apache/iggy)
- [TDengine](https://docs.tdengine.com/)
- [jsonwebtoken](https://github.com/Keats/jsonwebtoken)
- [Argon2 RFC 9106](https://www.rfc-editor.org/rfc/rfc9106)

## 2. 组件逐项评估

### 2.1 Rust + Tokio + Axum

**选择理由**

- 内存安全、无 GC，适合长连接网关和高并发 API。
- Tokio 生态完整：TCP（JT808）、HTTP/WS、定时器、channel、spawn_blocking。
- Axum 0.8 与 Tower 中间件、WebSocket、State 提取器集成直接；本项目的 handler/路由规模适中。
- 单二进制部署，适合边缘节点/TSP 私有化交付。

**代价**

- 编译时间与二进制体积高于 Go。
- Axum 0.8 生态仍在演进，升级需关注 extractor/route 语法变化。
- 团队需要熟悉 `Send + 'static`、借用和 async 生命周期。

**结论**：保留。对当前 TSP 的 HTTP + TCP + 定时任务组合，Rust/Axum 是合理选择；不建议为“更快上手”换 Go/Node，因为协议网关和长期运行稳定性更重要。

### 2.2 SQLx `AnyPool` + 三数据库

**现状**

- 同一套 repository 支持 SQLite（本地/演示）、PostgreSQL（集群）、MySQL/MariaDB（兼容部署）。
- `?N` 占位符在运行时改写为 `$N`/`?`；MySQL 重复参数需要展开。
- 迁移、DDL、行映射都做了方言适配。

**优点**

- 演示/边缘可用 SQLite，零外部依赖。
- 交付环境若已有 MySQL，可降低运维阻力。
- 单套代码覆盖三种后端，测试可跑 in-memory SQLite。

**风险**

- `AnyPool` **没有编译期 SQL 检查**，列名/类型错误只能在运行时发现。
- `Any` 对 `BOOL`、`NUMERIC`、`JSONB`、数组、`INTERVAL` 等类型表达能力有限，项目已用 `INTEGER`/TEXT 规避。
- 三套方言意味着索引、锁、`ON CONFLICT`、迁移行为都要手工验证；MySQL 的 `CREATE INDEX IF NOT EXISTS` 等差异已经需要特殊处理。
- 1M 基准显示列表 `COUNT(*)` + offset 和 dashboard 全表聚合是瓶颈；AnyPool 不是根因，但限制了针对性优化（如 PG 并行/覆盖索引/分区）。
- `AnyPool` 连接池参数单一，难以按 PostgreSQL 场景调优。

**替代方案**

| 方案 | 优点 | 代价 | 适用 |
| --- | --- | --- | --- |
| SQLx `PgPool` + `query!` | 编译期校验、PG 类型完整、性能优化直接 | 失去 SQLite/MySQL 一套代码 | 生产默认推荐 |
| SeaORM/Diesel | 模型/迁移完整 | 学习成本、动态查询/多方言仍有坑 | 中大型团队 |
| 保留 AnyPool | 兼容性最好 | 技术债持续 | 演示/边缘/过渡期 |

**结论**：保留 AnyPool 作为 **demo/兼容层**；生产主路径建议新增 `PgPool` repository 或 feature flag，逐步把核心查询迁到编译期校验。不要让“三库兼容”阻止 PostgreSQL 专属优化（分区、覆盖索引、并行聚合）。

### 2.3 PostgreSQL + PostGIS

**选择理由**

- 业务数据强一致、事务、复杂查询、审计都适合关系库。
- PostGIS 在同一库内提供 `geometry(Point,4326)`、GiST、`ST_DWithin`/`ST_Intersects`，避免额外空间服务。
- 轨迹、状态、围栏、告警可以在一个事务边界内处理。
- 已有实测：1M `vehicle_state` 下 bbox 查询约 119ms（debug 基准），空间索引可用。

**风险**

- PostGIS 扩展需要 DBA 权限；本机已验证自动检测/降级，但生产必须预装。
- 几何列与 `lat/lon` 双写增加存储与写入成本；当前用 `lat/lon` 兼容 SQLite/MySQL。
- 大表需要按时间分区，`VACUUM/ANALYZE` 必须纳入运维。
- `geography` 距离计算比平面几何更贵；nearby 1M 场景约 1.15s，需要限流/分页/预过滤。

**替代方案**

| 方案 | 评价 |
| --- | --- |
| PostGIS | 首选；事务内空间查询最强 |
| Elasticsearch geo | 检索强，但事务/一致性弱，增加一套系统 |
| Redis GEO | 适合附近粗查，不适合轨迹/围栏事务 |
| 应用层 Haversine | 仅小规模/降级；1M 不可行 |

**结论**：PostGIS 保留为生产空间主库；SQLite/MySQL 使用 lat/lon 降级是合理兼容策略，但不要宣称空间能力等价。

### 2.4 Redis 集群协调

**现状**

- 会话归属、节点存活、scheduler leader lease、Pub/Sub 事件。
- 使用 `ConnectionManager`，带重连；leader lease 用 Lua/脚本/TTL。
- 业务数据不写 Redis。

**优点**

- 轻量、低延迟；会话/租约/PubSub 是成熟用法。
- 与 Rust `redis` 客户端集成直接。

**风险**

- Redis 是集群协调的单点依赖；生产需要 Sentinel/Cluster/托管高可用。
- Pub/Sub 不持久化、无消费确认；节点断线会丢实时事件。
- 默认未强制 TLS/AUTH；跨网络部署必须补齐。
- 会话 TTL 与网关重连窗口需要压测；当前 90s 是策略值，不是 SLA。

**结论**：保留 Redis 做**短期协调 + 实时广播**；不要把 Redis Pub/Sub 当审计/回放日志。

### 2.5 OpenRaft 控制面

**现状**

- 只保存集群元数据/scheduler leader，不保存业务数据。
- `openraft-memstore` 类型 + 自定义 `FileLogStore`/`FileStateMachine`（JSON 文件）。
- HTTP JSON RPC，`TSP_RAFT_DATA_DIR` 持久化。

**优点**

- 控制面小，故障不影响业务数据。
- 选主/租约逻辑独立于 Redis。

**风险**

- 文件 JSON 日志/状态机的可靠性、并发、fsync、压缩、快照都需要长期验证；不适合作为关键控制面唯一依据。
- 自研 Raft 网络层/成员变更/恢复流程复杂；`openraft-legacy` 直接依赖未使用，增加攻击面。
- 三节点启动时曾出现迁移/锁竞争和 Raft heartbeat 超时，运维复杂度高。
- K8s 场景已有 Lease/etcd，重复造轮子。

**结论**：

- 演示/单机可保留。
- 生产优先 **K8s Lease 或 etcd** 做 leader election；若坚持 Raft，使用可靠存储（RocksDB/ sled/ SQL）并做故障注入。
- 明确“Raft 不是业务数据复制方案”。

### 2.6 事件总线：Redis Pub/Sub / Iggy / Walrus

**现状**

- 默认：Redis Pub/Sub（实时 WebSocket 扇出）。
- `TSP_IGGY_URL`：Iggy HTTP 持久事件日志，每节点独立 consumer offset。
- `TSP_WALRUS_ADDR`：Walrus TCP 下游归档，明确不用于多节点广播（共享 cursor）。

**问题**

- 三套通道语义不同：Pub/Sub 不持久；Iggy 持久但适配器基于 HTTP 手写协议；Walrus 只是下游日志。
- 同时开启会造成事件重复、顺序/去重责任转移到消费端。
- Iggy 适配器需要跟随 Iggy REST API 版本；未锁 SDK 版本。
- Walrus 协议简单，但生态/运维资料少。

**建议**

- **默认只保留 Redis Pub/Sub + 业务库**，满足 10K–100K 实时看板。
- 需要事件回放/跨区域时，选 **Iggy 或 Kafka/NATS** 其中之一，作为唯一 durable bus；Redis 退回协调。
- Walrus 从默认交付中移除或标记实验性；除非已有 Walrus 运维体系。
- 定义事件 envelope（id、origin、ts、type、version、trace_id）和幂等键；消费者去重。

### 2.7 时序/轨迹存储：PostGIS / tsink / TDengine

**现状**

- 权威轨迹在 `telemetry` 表（PostgreSQL）。
- 可选 tsink：Prometheus 文本导入 + PromQL 查询。
- 可选 TDengine：REST SQL，超级表 + 子表。
- 可选 PostGIS：空间列/索引。

**问题**

- 三套轨迹存储重叠，冷热分层/一致性/回放语义未统一。
- 双写失败时没有补偿/对账；`tsink`/TDengine 查询结果与 PG 权威数据可能不一致。
- tsink 适配器是自定义 HTTP 契约，成熟度/社区支持需评估。
- TDengine REST 每次查询走 HTTP，和 PG 事务边界不同。

**建议**

- 10K–100K：**PostgreSQL/PostGIS 单栈**，按时间分区 + 保留策略即可；不要默认双写。
- 100K+ 或长周期轨迹分析：选 **TDengine 或 TimescaleDB** 之一做冷/温存储；PG 保留近期权威数据。
- tsink 仅作为实验/特定 Prometheus 环境适配，不作为默认。
- 若启用双写，增加 outbox/重试/对账指标，明确“PG 权威、TSDB 可重建”。

### 2.8 认证与密钥

**现状（已加固）**

- Argon2id 密码哈希。
- JWT RS256 + JWKS（`/.well-known/jwks.json`），access token 默认 15 分钟。
- refresh token 7 天、服务端仅存 SHA-256、刷新轮换、登出吊销。
- access token 按 `jti` 写 Redis 吊销键，单机降级进程内缓存；跨节点登出立即 401。
- 登录失败审计 + 15 分钟 5 次限流。

**剩余风险**

- 每个已认证请求多一次 Redis `EXISTS`（`jti` 吊销检查），高 QPS 需本地缓存/布隆过滤器。
- JWKS 当前单 key，尚未实现多 key 轮换。
- 无 OIDC/SAML、MFA；JWT 仍在浏览器 localStorage，需 CSP/XSS 防护。
- Redis 不可用时吊销检查 fail-closed。

**建议**

- 企业客户接 OIDC/SAML；保留本地账号 fallback。
- 实现 JWKS 多 key/轮换与旧 key 保留窗口。
- 访问 token 吊销检查可加本地 TTL 缓存，降低 Redis 压力。

### 2.9 协议与标准

**现状**

- JT/T 808：注册/鉴权/心跳/位置/0x0704 批量/部分下行/0x0001 应答；分包重组支持；加密报文拒绝。
- JT/T 809：传输信封 + 登录体子集。
- GB/T 32960：传输头 + 实时数据前 20 字节/登录体子集。
- 均为 codec seam，不是全量合规。

**风险**

- 明文 TCP；无 TLS/mTLS；终端接入依赖专网/VPN。
- 标准版本/地方标准差异未覆盖；未做部标平台对接认证。
- 0x0704 批量重复点预检查已优化为一次 IN，但终端重传/乱序仍需幂等策略。

**建议**

- 在合同/文档中明确“子集实现”，不要使用“符合/认证”措辞。
- 生产网关放 TLS 终止/专网；增加终端证书/白名单。
- 建标准符合性测试集（帧样本、异常帧、分包、加密）后再谈认证。

### 2.10 可观测性与部署

**现状**

- `/health`、`/metrics`（Prometheus 文本）、`tracing` 日志、`/cluster/status`。
- K8s StatefulSet + PVC 示例；3 节点 Raft/Redis 部署。

**缺口**

- 无 OpenTelemetry traces/metrics exporter；无 Prometheus 告警规则/Grafana dashboard。
- 无 Helm/Kustomize；无 PDB/NetworkPolicy/资源 requests/limits 的完整清单。
- 无备份/恢复演练、RPO/RTO 定义。
- 1M 基准是 debug 单节点，不是生产容量承诺。

**建议**

- 增加 OTel/ Prometheus 指标（HTTP、DB pool、telemetry ingest、Raft、Redis）+ 告警。
- 交付 Helm chart，补 PDB/NetworkPolicy/backup CronJob。
- release 构建做容量基线；明确单节点/三节点/多租户隔离的 SLO。

## 3. 选型决策矩阵

评级：★★★ 推荐，★★ 可用，★ 谨慎。

| 领域 | 选择 | 备选 | 结论 |
| --- | --- | --- | --- |
| 语言/运行时 | Rust + Tokio ★★★ | Go/Node | 保留；协议网关/长期运行优先 |
| HTTP | Axum ★★★ | Actix Web/Poem | 保留；团队熟悉 Tower 生态 |
| 数据访问 | SQLx AnyPool ★★ | PgPool+query! / SeaORM | 演示保留；生产加 PgPool |
| 主库 | PostgreSQL ★★★ | MySQL | 生产默认 PG |
| 空间 | PostGIS ★★★ | ES/Redis GEO | 保留，事务内空间查询 |
| 协调 | Redis ★★★ | etcd/K8s Lease | 保留；HA/TLS/AUTH 必配 |
| 控制面选主 | OpenRaft ★★ | etcd/K8s Lease | 仅演示；生产优先 Lease/etcd |
| 实时事件 | Redis Pub/Sub ★★★ | Iggy/Kafka/NATS | 默认保留 |
| 持久事件 | Iggy ★★ | Kafka/NATS | 仅在需要回放时启用，且二选一 |
| 下游归档 | Walrus ★ | Kafka/S3 | 默认移除/实验性 |
| 轨迹主存 | PG/PostGIS ★★★ | TDengine/Timescale | 10K–100K 默认 |
| 轨迹分析 | TDengine ★★ | Timescale/ClickHouse | 100K+ 再引入，二选一 |
| tsink | ★ | Prometheus/TDengine | 实验性，不默认 |
| 认证 | Argon2id+JWT ★★ | OIDC/Keycloak | 补 RS256/refresh/吊销/OIDC |
| 协议 | 自研 JT808 子集 ★★ | 商业网关 | 明确子集；补 TLS/认证测试 |

## 4. 风险登记

| 风险 | 严重度 | 影响 | 缓解 |
| --- | --- | --- | --- |
| 可选组件重叠/双写不一致 | 高 | 事件重复、轨迹不一致、运维复杂 | 默认单栈；二选一；outbox/对账 |
| SQLx AnyPool 无编译期 SQL 校验 | 高 | 运行时列/类型错误；难优化 | 生产 PgPool+query!；CI 集成测试 |
| OpenRaft 文件 JSON 存储 | 高 | 控制面恢复/脑裂风险 | etcd/K8s Lease；可靠 Raft 存储；故障注入 |
| Redis 单点/无 TLS | 高 | 会话丢失、集群不可用、窃听 | HA、TLS、AUTH、网络隔离 |
| JWT HS256/12h/无吊销 | 高 | token 伪造、长期有效 | RS256/JWKS、短 TTL、refresh、jti |
| JT808 明文 TCP | 高 | 终端伪造/窃听 | 专网/VPN、TLS 终止、证书/白名单 |
| PostGIS 扩展缺失/备份遗漏 | 中 | 空间查询降级、恢复失败 | DBA 预装、备份含扩展、演练 |
| MySQL 方言差异 | 中 | 迁移/索引/锁行为不一致 | 只在支持矩阵内承诺；PG 为主 |
| 1M 列表/dashboard 瓶颈 | 高 | RPS 低、首屏慢 | keyset、预聚合、分区、读副本 |
| 标准子集被误认为合规 | 高 | 合同/验收风险 | 明确范围；符合性测试/认证 |
| 依赖升级滞后（rand 0.8/native-tls） | 中 | 安全/兼容 | cargo audit/deny、定期升级、TLS 统一 rustls |

## 5. 分场景推荐

### 5.1 演示 / 边缘单机

```text
SQLite + 单进程 Axum + 本地事件 bus
不启用 Redis/OpenRaft/Iggy/Walrus/TDengine
可选 PostGIS=false
```

优点：零外部依赖，交付简单。当前 demo 已满足。

### 5.2 生产 10K–100K 在线

```text
PostgreSQL 16 + PostGIS
Redis HA（会话/租约/PubSub）
2+ API 节点 + 独立 JT808 网关
K8s Deployment/StatefulSet + PDB/NetworkPolicy
单一时序主存：PG/PostGIS（按时间分区）
事件：Redis Pub/Sub；需要回放时加 Iggy（唯一 durable bus）
控制面：K8s Lease/etcd 优先；OpenRaft 仅过渡
认证：OIDC 或 RS256 JWT + refresh/吊销
可观测：Prometheus + OTel + 告警 + 备份演练
```

这是当前代码最匹配的区间。

### 5.3 1M 在线

在 5.2 基础上增加：

- 车辆列表 keyset + 总数缓存；dashboard 预聚合表。
- PG 分区/读副本；轨迹冷热分层到 TDengine/Timescale（二选一）。
- 多 API/网关分片；限流/熔断；nearby 分页/距离上限。
- 事件总线升级为 Kafka/NATS/Iggy 单一 durable 方案。
- 独立容量测试（release、生产硬件）后再承诺 SLO。

## 6. 建议的落地顺序

### P0（生产前必须）

1. 冻结默认组件清单：Redis + PostgreSQL/PostGIS + 单事件通道；Iggy/Walrus/tsink/TDengine 改为显式可选。
2. 生产数据访问切 PgPool/编译期 SQL；AnyPool 仅 demo/兼容。
3. JWT 改 RS256/JWKS + 短 TTL + refresh/吊销；补 Redis TLS/AUTH、JT808 专网/TLS。
4. 控制面选型：K8s Lease/etcd，或为 OpenRaft 换可靠存储并做故障注入。
5. 增加备份/恢复、Prometheus 告警、OpenAPI/契约测试。

### P1（规模化）

1. 列表 keyset/总数缓存；dashboard 预聚合。
2. PG 分区/读副本；轨迹冷热分层，二选一 TSDB。
3. 事件 envelope/幂等/outbox；Iggy 或 Kafka/NATS 单栈。
4. release 构建容量基线；1M 生产硬件验证。

### P2（清理）

1. 移除未使用的 `openraft-legacy`；`cargo machete`/`cargo deny` 纳入 CI。
2. 统一 TLS 后端（SQLx rustls vs reqwest native-tls）。
3. 升级 `rand`、评估 `jsonwebtoken`/`redis` 新版本。
4. 将可选适配器改为 Cargo features，减少默认攻击面。

## 7. 最终建议

**保留**：Rust/Tokio/Axum、PostgreSQL+PostGIS、Redis（协调）、SQLx（过渡期 AnyPool）、Argon2id+JWT（加固后）。

**默认不启用**：Walrus、tsink、TDengine、Iggy、OpenRaft（生产控制面）。它们可以作为能力预留，但不应出现在默认交付拓扑。

**最值得投入的三件事**：

1. 生产数据访问从 `AnyPool` 收敛到 PostgreSQL 专用路径。
2. 控制面从自研 Raft/JSON 存储收敛到 K8s Lease/etcd。
3. 把 1M 基准暴露的列表/看板/nearby 瓶颈用预聚合、keyset、分区解决。

---

附：容量数据见 [BENCHMARK_REPORT.md](BENCHMARK_REPORT.md)，代码结构见 [STRUCTURE.md](STRUCTURE.md)，PostGIS 见 [POSTGIS.md](POSTGIS.md)。
