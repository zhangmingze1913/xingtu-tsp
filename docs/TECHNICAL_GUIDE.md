# 星途 TSP 技术文档

## 1. 技术栈

```text
Rust 2021 / 1.94.1+
Axum 0.8 / Tokio / Tower-HTTP
SQLx 0.8 AnyPool
PostgreSQL 16 + PostGIS / KingbaseES V8R6/V9 + KGIS / MySQL / SQLite
Redis 8（集群会话、租约、Pub/Sub）
OpenRaft 0.10（仅控制面）
RustFS（S3 兼容录制归档）
TDengine 3.x（可选时序后端）
```

## 2. 代码结构

```text
src/
  lib.rs                 进程启动、路由装配、生命周期
  bin/                   可执行入口
  bootstrap/             配置校验、DB/集群/存储启动
  config.rs              环境变量配置
  api/                   HTTP handlers、鉴权提取器、WebSocket
  services/              领域服务 + maintenance 后台任务
  db/                    数据访问、迁移、空间、PII
  models/                API/领域模型
  jt808/                 JT/T 808 编解码、网关、控制
  standards/             JT/T 809、GB/T 32960、JT/T 1078、GB/T 19056
  media/                 媒体会话、RustFS 归档、spool、播放缓存
  cluster/               Redis 协调、OpenRaft 封装
  raft/                  Raft 控制面
  security/              PII、字段策略、密钥
  tdengine.rs            TDengine REST 适配器
  timeseries.rs          时序后端路由
  vault_keys.rs          Vault KV v2 密钥同步
```

### 2.1 本次结构优化

TDengine 告警模块已拆出：

```text
src/tdengine.rs            客户端、写入、查询、统计
src/tdengine/alert.rs      告警 payload、HMAC 签名、重试、冷却
```


`lib.rs` 原来包含约 450 行后台任务，已迁至：

```text
src/services/maintenance.rs
```

迁移内容包括：

- archive spool replay / cleanup
- offline / retention sweeper
- JT808 downlink timeout sweeper
- TDengine reconciler
- reconciliation history pruner
- export job cleanup / recovery
- Vault key sync

结果：

```text
lib.rs          696 -> 244 行
maintenance.rs  新增 461 行

api/integrations.rs
  -> api/integrations/mod.rs      对账/历史/CSV
  -> api/integrations/export.rs   导出任务/令牌/失败告警
```

入口文件只保留：

- 应用装配
- 路由/状态
- 启动调用
- 优雅关闭

### 2.2 仍需关注的结构问题

| 文件 | 行数 | 问题 | 建议 |
| --- | ---: | --- | --- |
| `standards/jt809.rs` | ~4.3k | 帧/加密/业务/测试集中 | 拆 `frame`/`crypto`/`business` |
| `standards/gb32960.rs` | ~3.5k | 编解码与测试集中 | 拆 `codec`/`state`/`tests` |
| `jt808/control.rs` + `control/media.rs` | 2658 + 327 | 媒体控制已拆出；其余控制仍集中 | 可按参数/事件/区域继续拆 |
| `api/integrations/` | mod ~0.5k + export ~0.7k | 已拆出导出任务/令牌/告警 | 可继续拆 `reconcile` |
| `tdengine.rs` + `tdengine/alert.rs` | 823 + 260 | 已拆出告警/冷却/签名；核心写入与查询仍在 | 可继续拆 client/writer/query |

这些是后续可维护性优化项，不影响当前功能。

## 3. 请求链路

```text
HTTP
 -> Axum Router
 -> AuthUser extractor (JWT RS256)
 -> handler
 -> Db / TdengineStore / media
 -> JSON / CSV / stream
```

关键约束：

- PostgreSQL/KingbaseES 是业务权威源
- SQLite 用于单机/测试
- TDengine 只是可选查询副本
- Redis 不保存业务真相
- OpenRaft 只管理控制面

## 4. 数据库抽象

`Db` 使用 SQLx `AnyPool`，`Backend` 区分：

```text
Sqlite | Postgres | Mysql
```

KingbaseES 复用 PostgreSQL wire 路径，并用 `Db::kingbase` 标记：

- URL：`kingbase://` / `kingbase8://` / `kingbasees://`
- 规范化为 `postgres://` 供 SQLx 连接
- `engine_name()` 返回 `kingbase`
- 健康检查输出 `database_engine=kingbase`

### 4.1 Kingbase 兼容点

- 默认镜像 `DB_MODE=oracle`，空字符串会变 NULL；必须 `DB_MODE=pg`
- `LEVEL` 是保留字：Kingbase 下 `Db::sql` 将 `level` 引号化为 `"level"`，schema 同样处理
- 跳过 `INCLUDE` 覆盖索引
- advisory lock 失败时 warning 继续
- KGIS 扩展尝试 `kingbase_spatial` / `postgis`
- KGIS 不可用时降级 lat/lon

### 4.2 迁移

`schema_migrations` 记录版本；启动时幂等执行。

```text
0001 baseline
...
0023 export jobs
0024-0027 cursor/lease
0028-0029 download tokens
0030 export alert state
```

## 5. 遥测与空间

入口统一：

```text
LocationSample -> services::telemetry::ingest -> Db::ingest_location
```

单事务内更新：

- telemetry
- vehicle_state
- trips
- geofence_states
- alerts

空间字段：

```sql
geom geometry(Point,4326)
```

PostGIS/KGIS 可用时：

- `ST_DWithin(geography...)`
- `ST_Intersects(geom, ST_MakeEnvelope(...))`

否则：

- 读取 lat/lon
- Haversine / bbox 内存过滤

## 6. 时序后端

```text
TSP_TIMESERIES_BACKEND=postgres|tdengine|dual
```

- `postgres`：默认
- `tdengine`：必须配置 URL，启动失败即失败
- `dual`：双写，TDengine 故障降级 PG

TDengine 特性：

- 有界队列、重试、spool、重放
- PG/TDengine 对账
- 历史/CSV/任务化导出
- 告警 webhook、HMAC、Alertmanager、冷却、跨实例 claim
- Grafana 面板

详见 [TIMESERIES_BACKENDS.md](TIMESERIES_BACKENDS.md)。

## 7. 媒体归档

```text
JT/T 1078 -> media hub -> RustFS S3
```

- 关键帧对齐录制
- H.264 Annex-B / AAC ADTS 原始对象
- 播放时转封装为 fMP4
- 上传重试 + 本地 spool
- 保留策略与 LRU 播放缓存

详见 [RUSTFS.md](RUSTFS.md)。

## 8. 安全

- 密码 Argon2
- JWT RS256 + refresh token 轮换 + jti 吊销
- RBAC：admin/operator/viewer
- 组织隔离
- PII 字段加密与批量 hydration
- JT808 TLS/mTLS
- 导出 AES-256-GCM + key id + Vault/文件密钥
- 下载令牌 jti 单次消费

详见 [SECURITY.md](SECURITY.md)。

## 9. 配置

最小单机：

```bash
TSP_BIND=127.0.0.1:8090
TSP_DATABASE=data/tsp.db
TSP_DEMO_MODE=true
TSP_JWT_SECRET=change-me
```

PostgreSQL：

```bash
TSP_DATABASE='postgres://tsp:***@pg:5432/tsp?sslmode=disable'
TSP_POSTGIS_ENABLED=true
```

KingbaseES：

```bash
TSP_DATABASE='kingbase://tsp:***@kes:54321/tsp?sslmode=disable'
TSP_POSTGIS_ENABLED=true
```

完整变量见 `.env.example`、`src/config.rs` 和 [OPERATIONS_GUIDE.md](OPERATIONS_GUIDE.md)。

## 10. 构建与测试

```bash
cargo build --release
cargo test --lib
cargo test --test auth --test vehicles --test standards --test storage --test cluster \
  --test standards_conformance --test jt809_sim --test jt809_tls_sim \
  --test gb32960_sim --test jt1078_sim --test pii_vault --test standards_fuzz
```

当前基线：

```text
lib 299 passed
integration 43 passed
fmt clean
clippy --all-targets clean
```

## 10.1 可选存储/中间件微测试

已覆盖：

```text
Iggy          HTTP login/send/poll/commit 语义（mock）
Walrus        REGISTER/PUT TCP 帧（mock TCP）
tsink         Prometheus import 文本、Bearer、失败状态（mock HTTP）
Redis         session SET/GET（真实容器，环境变量门控）
RustFS        上传/分页/清理/spool（S3 mock）
TDengine      query/range/retry/spool（REST mock）
Vault         KV v2/AppRole/K8s login（mock HTTP）
```

Redis 真实测试：

```bash
docker run -d --name tsp-test-redis -p 56379:6379 redis:7-alpine
TSP_TEST_REDIS_URL=redis://127.0.0.1:56379/8 cargo test --lib redis_session_roundtrip
```

MySQL 容器测试：

```bash
docker run -d --name tsp-mysql -e MYSQL_ROOT_PASSWORD=root \
  -e MYSQL_DATABASE=tsp -e MYSQL_USER=tsp -e MYSQL_PASSWORD=tsp-dev-password \
  -p 33060:3306 mysql:8.0
TSP_TEST_MYSQL_URL='mysql://tsp:tsp-dev-password@127.0.0.1:33060/tsp' \
  cargo test --lib mysql_connection_smoke
```

Kingbase 真实测试：

```bash
TSP_TEST_KINGBASE_URL='kingbase://...' cargo test --lib kingbase_connection_smoke
```

没有设置环境变量时，这些测试会跳过，不会伪造通过。

## 11. 扩展点

| 需求 | 位置 |
| --- | --- |
| 新协议接入 | `standards/` + `jt808` 适配器 |
| 新数据库 | `db/mod.rs` Backend + schema |
| 新时序库 | `timeseries.rs` + adapter |
| 新对象存储 | `media/archive.rs` |
| 新后台任务 | `services/maintenance.rs` |
| 新 API | `api/` + `router.rs` |

扩展时保持：

1. `LocationSample` 与协议解耦
2. 业务写入单事务
3. 外部存储可降级
4. 新能力必须有测试与真实环境边界说明
