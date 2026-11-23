# 性能优化记录

测试环境：Windows + Rust debug 构建、PostgreSQL 55432、Redis 6379/8、3 节点 Raft、18 台 demo 车辆。
HTTP 基准：Python 线程池，30 次顺序 + 100 次 20 并发；telemetry 为 10 并发、30 个唯一样本。
debug 构建不代表 release 绝对值，但可用于同环境前后对比。

## 1. 优化项

| 项 | 改动 | 说明 |
| --- | --- | --- |
| dashboard 缓存 | `AppState` 增加按组织 2 秒 TTL 的 JSON 缓存 | 遥测写入时 `dashboard_cache.clear()`；最多 2 秒陈旧 |
| 数据库索引 | `0004` vehicles(org_name)、`0005` geofences(org_name)、`0006` terminals(status,last_heartbeat_at) | 同时写入 `schema/*.sql` 供新库使用 |
| Argon2 | 登录校验改为 `tokio::task::spawn_blocking` | 避免 CPU 密集密码校验阻塞 async worker |
| telemetry 查询 | 车辆 + 当前状态合并为一次 LEFT JOIN | 每个定位点少一次 DB 往返 |
| 0x0704 批量 | `ingest_locations` 单事务；网关批量解析后一次提交 | N 个点从 N 个事务降为 1 个事务 |
| 围栏状态 | 先批量读 prior，再单条多值 upsert | 从约 2×N 次 SQL 降为 2 次 |
| dashboard 趋势 | 7 天趋势 14 个查询改为 `join_all` 并行 | 冷启动减少串行等待 |
| dashboard 标量 | 11 个标量统计合并为一次多子查询 SELECT | 11 次往返降为 1 次，再与状态分布并发 |
| 0x0704 重复点 | 一次 `IN (...)` 查询替代逐点查询 | N 次查询降为 1 次 |
| prepared cache | SQLx query `.persistent(true)` | 围栏动态 upsert 等复用 prepared statement |
| 轨迹索引 | PostgreSQL 覆盖索引 `idx_telemetry_vehicle_ts_cover` | 10 万级轨迹查询稳定化 |
| 连接池 | 曾尝试 10→20，压测抖动变大，已回退 10 | 避免过度并发打满 PG/CPU |
| dashboard 路由 | 直接返回缓存 JSON `Response` | 命中时不再反序列化/序列化 `DashboardOverview` |

## 2. 优化前后

| 接口 | 指标 | 优化前 | 优化后 |
| --- | --- | --- | --- |
| `/api/dashboard` | 顺序 p50 | 33.8 ms | **2.0 ms** |
| `/api/dashboard` | 顺序 avg | 40.8 ms | **5.3 ms** |
| `/api/dashboard` | 20 并发 p50 | 150.0 ms | **~16 ms** |
| `/api/telemetry` | 10 并发 p50 | 50.6 ms | **32.0 ms** |
| `/api/vehicles` | 顺序 p50 | 7.6 ms | 6.7 ms（debug 波动） |

### release 构建复测

| 接口 | 顺序 p50 | 20 并发 p50 |
| --- | --- | --- |
| `/api/vehicles` | 3.2 ms | 9.8 ms |
| `/api/dashboard` | 1.6 ms | 5.1 ms |
| `/api/alerts` | 3.7 ms | 10.7 ms |
| `/api/telemetry`（10 并发） | 33.2 ms | — |

说明：

- dashboard 命中 2 秒缓存时接近纯 HTTP/JSON 成本。
- telemetry 仍受事务、围栏状态 upsert、告警规则查询影响；p95 在 debug/并发下波动较大。
- 登录 debug 下单次 Argon2 约 0.7–1.1s；`spawn_blocking` 的目标是避免阻塞其他请求，不代表密码校验本身变快。release 构建会显著降低。

## 2.1 EXPLAIN 结果

PostgreSQL `EXPLAIN (ANALYZE, BUFFERS)`：

- 18 台车规模下 dashboard 车辆统计：Seq Scan，0.58 ms；状态聚合 0.30 ms。
- alerts active 统计：Seq Scan + Hash Join，0.60 ms。
- telemetry 按 `ts >=` + `ORDER BY ts DESC LIMIT 1000`：走 `idx_telemetry_ts`，约 3.7 ms / 1000 行；按 vehicle+ts 的轨迹查询使用 `idx_telemetry_vehicle_ts`。
- 当前数据量小，规划器选择 Seq Scan 合理；索引主要服务于数据增长后的 org/telemetry/alert 路径。

## 3. 缓存一致性

- key：`org_name`（管理员 `*`，operator/viewer 本组织）。
- TTL：2 秒。
- 失效：每次 telemetry 成功入库后清空全部 dashboard 缓存。
- 影响：dashboard 最多 2 秒陈旧；车辆/告警/指令写操作不强制清空，依赖 TTL。

## 3.1 10 万级容量

详见 [CAPACITY.md](CAPACITY.md)：

- `telemetry` 148,669 行；`veh_001` 108,207 行
- 轨迹 7 天 + LIMIT 1000：`VACUUM (ANALYZE)` 后 2.9ms
- dashboard 冷启动 73.9ms，缓存命中 3.9ms
- 轨迹 API debug 90–168ms（主要 JSON 序列化）

## 4. 后续可继续

1. dashboard 剩余标量查询进一步并行化。
2. 围栏 upsert 的占位符 SQL 可改为 prepared statement 缓存。
3. release 构建下做更大数据量（10 万 telemetry）容量测试。
4. release 构建 + PG `EXPLAIN (ANALYZE, BUFFERS)` 做真实容量测试。
5. 登录增加每用户令牌/会话缓存需谨慎，不建议缓存明文密码校验结果。
