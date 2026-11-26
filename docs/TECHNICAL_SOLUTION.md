# 星途 TSP 技术方案

## 1. 目标与边界

目标：

- 统一接入车辆/T-Box 位置、告警、指令与媒体。
- 提供可运行的 TSP 垂直切片：车辆、轨迹、围栏、告警、远控、看板。
- 支持 PostgreSQL、MySQL、SQLite、KingbaseES 多后端。
- 支持可选 TDengine/tsink、RustFS、Iggy/Walrus。
- 可单机演示，也可多副本部署。

不是：

- 已通过 JT/T 808 全量或第三方合规检测的产品。
- 已通过生产容量/高可用验证的系统。
- 可直接用于监管备案或无人值守远程车控的系统。

## 2. 关键设计决策

| 决策 | 选择 | 理由 | 代价 |
| --- | --- | --- | --- |
| 语言 | Rust + Tokio | 内存安全、并发、单二进制 | 开发成本高 |
| HTTP | Axum 0.8 | 类型安全、Tower 生态 | 生态相对年轻 |
| 数据访问 | SQLx AnyPool | 多后端、异步 | 部分类型需 cast |
| 权威库 | PostgreSQL/KingbaseES | 事务、索引、PostGIS | 运维复杂 |
| 时序 | PG 默认，TDengine 可选 | 小规模简单，大规模可扩展 | 双写一致性 |
| 消息 | Redis Pub/Sub + 可选 Iggy/Walrus | 实时与持久分离 | 组件多 |
| 控制面 | OpenRaft | leader/元数据 | 仅控制面，不复制业务表 |
| 媒体 | RustFS S3 | 原始 H.264/AAC 归档 | 需对象存储 |
| 空间 | PostGIS/KGIS + lat/lon 降级 | 能力与可运行性兼顾 | 两套路径 |

## 3. 数据流

### 3.1 定位

```text
T-Box/HTTP/Simulator
  -> LocationSample
  -> ingest transaction
     -> telemetry
     -> vehicle_state
     -> trips
     -> geofence_states
     -> alerts
  -> dashboard cache clear
  -> WebSocket
  -> optional TDengine/tsink
```

事务边界：业务表在同一事务；外部存储不参与事务。

### 3.2 查询

```text
trajectory -> tsink -> TDengine -> PostgreSQL fallback
nearby/bbox -> PostGIS/KGIS -> lat/lon fallback
dashboard -> 2s org cache -> aggregate SQL
```

### 3.3 远控

```text
API -> command row(pending) -> JT808 encode -> gateway send
    -> terminal reply -> command status
```

终端离线/超时不会伪造成成功；下行超时有自动重试与审计。

## 4. 数据模型

核心表：

```text
users, vehicles, vehicle_state, terminals
telemetry, trips, geofences, geofence_states
alerts, commands, audit_logs, pii_vault
schema_migrations
timeseries_reconciliations, reconciliation_export_jobs
export_download_tokens, export_alert_state
```

空间：

```sql
telemetry.geom geometry(Point,4326)
vehicle_state.geom geometry(Point,4326)
```

索引：

- `telemetry(vehicle_id, ts)`
- `telemetry(ts)`
- `telemetry USING gist(geom)`
- `telemetry USING gist((geom)::geography)`
- `vehicles(org_name, plate_no)`
- `alerts(acknowledged, ts)`

## 5. 多数据库适配

| 后端 | 说明 |
| --- | --- |
| PostgreSQL | 主推，PostGIS |
| KingbaseES | PostgreSQL wire，KGIS；必须 `DB_MODE=pg` |
| MySQL | `?` 占位符转换、DDL 幂等 |
| SQLite | 单机/测试 |

Kingbase 兼容：

- URL `kingbase://` -> `postgres://`
- `LEVEL` 保留字引号化
- 跳过 `INCLUDE` 索引
- advisory lock 失败降级
- KGIS 缺失时 lat/lon 降级

MySQL 兼容：

- `schema_migrations.version` 用 `VARCHAR(64)`
- 无 `CREATE INDEX IF NOT EXISTS`，启动容忍 1061
- 基础 schema 不重复建迁移索引

## 6. 时序与对账

```text
TSP_TIMESERIES_BACKEND=postgres|tdengine|dual
```

- `postgres`：默认
- `tdengine`：启动探测失败即失败
- `dual`：双写，TDengine 失败降级

写入：

- 有界队列
- 重试
- 本地 spool
- 重放
- 指标：queue depth/failures/spool

对账：

- 定时统计 PG 与 TDengine
- 不一致写历史
- webhook（HMAC、Alertmanager、冷却）
- 导出任务（分页、租约、恢复、加密、单次下载令牌）

## 7. 安全方案

- Argon2 密码
- JWT RS256/JWKS，refresh 轮换，jti 吊销
- RBAC：admin/operator/viewer
- 组织隔离
- PII 字段加密；批量 hydration
- JT808 TLS/mTLS + CIDR
- Redis TLS/AUTH
- 导出 AES-256-GCM + key id + 密钥轮换
- 下载令牌 HMAC + 单次消费

生产启动校验：

- 禁止默认 admin 密码
- 禁止 demo/模拟指令
- 禁止无 TLS JT808
- 禁止明文 Redis
- 要求持久 JWT 密钥

## 8. 性能设计

- dashboard 2s 缓存
- 车辆列表 PII 批量解密
- telemetry 单事务
- 0x0704 批量点单事务
- 围栏状态批量读取/upsert
- 轨迹覆盖索引
- 导出分页/任务化
- TDengine 异步写入
- RustFS 分段 + spool

已测：

```text
release KES 列表 p50 12.9ms，178 RPS
PostGIS 100万 bbox 348ms
PostGIS 100万 nearby 14.6s（全量 count）
```

## 9. 失败模式

| 故障 | 行为 |
| --- | --- |
| PostGIS/KGIS 缺失 | lat/lon 降级 |
| TDengine down（dual） | spool + PG 继续 |
| TDengine down（tdengine） | 启动失败 |
| RustFS down | spool，不阻塞业务 |
| Redis down | 单机 leader/本地广播 |
| Iggy down | 记录错误，事件不保证 |
| Vault 不可用 | 使用文件密钥/启动失败按配置 |
| 数据库 down | API 不可用，明确报错 |

## 10. 部署

- 单机 Compose：SQLite + RustFS
- 集群 Compose：PostgreSQL + Redis + 2 API + 2 网关
- K8s：Deployment/Service/HPA/PDB/NetworkPolicy
- KES：容器模板，`DB_MODE=pg`
- 可选 TDengine/RustFS/Iggy

## 11. 测试策略

```text
单元/协议：299 passed
集成：41 passed
真实容器：PG/PostGIS/MySQL/Redis/RustFS/TDengine/Vault/Iggy/Walrus/tsink
K8s：kind smoke
```

未验证：

- KGIS
- 自动 failover
- 多节点 Iggy/Walrus/tsink
- 真实终端/弱网
- 第三方标准检测

## 12. 风险与后续

| 风险 | 影响 | 建议 |
| --- | --- | --- |
| 100万 nearby 14.6s | 交互慢 | 时间窗/分区/TDengine |
| 协议大文件 | 维护难 | 拆 frame/codec |
| 多后端差异 | 隐性 bug | 每后端真实容器测试 |
| 无自动 failover | 故障恢复慢 | Patroni/云 RDS |
| 无 KGIS | 信创空间能力缺口 | 官方插件包 |
| 无标准认证 | 不能监管备案 | 第三方检测 |

## 13. 相关文档

- [ARCHITECTURE.md](ARCHITECTURE.md)
- [ARCHITECTURE_DIAGRAMS.md](ARCHITECTURE_DIAGRAMS.md)
- [OPERATIONS_GUIDE.md](OPERATIONS_GUIDE.md)
- [USER_GUIDE.md](USER_GUIDE.md)
- [PERFORMANCE_EVALUATION_2026-11-25.md](PERFORMANCE_EVALUATION_2026-11-25.md)
