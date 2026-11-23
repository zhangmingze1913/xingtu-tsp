# 10K / 100K / 1000K 车辆在线性能测试报告

## 1. 测试结论

| 规模 | 在线车辆 | dashboard 冷启动 | dashboard 缓存 | 列表 p50 | 地图 p50 | bbox p50 | nearby p50 | 列表吞吐 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 10K | 10,000 | 212 ms | 8.9 ms | 22.8 ms | 38.9 ms | 127 ms | 140 ms | 101 RPS |
| 100K | 100,000 | 1.57 s | 8.5 ms | 39.7 ms | 39.5 ms | 121 ms | 573 ms | 59 RPS |
| 1000K | 1,000,000 | 3.62 s | 7.6 ms | 281 ms | 60.4 ms | 119 ms | 1,152 ms | 4.6 RPS |

所有请求 **0 错误**。

结论：

- **10K、100K 在线**：交互接口均在几十毫秒到百毫秒级；dashboard 冷启动 0.2–1.6s，缓存命中约 9ms。
- **1000K 在线**：地图/bbox 仍保持 60–120ms；车辆分页单请求 281ms，但高并发下退化到约 0.9–1.1s、4.6 RPS，是当前最主要瓶颈。
- **dashboard 1M 冷启动 3.6s**，必须依赖 2 秒缓存；缓存命中 7.6ms。
- **nearby 1M 1.15s**，主要成本是 500 条结果 + 空间排序/距离计算；地图 bbox 已通过 limit 500 控制在 119ms。

## 2. 测试环境

```text
CPU       4 vCPU
内存      12 GB
OS        Windows
Rust      debug 构建（非 release）
API       单节点 TSP API（TSP_CLUSTER_MODE=false）
数据库    PostgreSQL 16.4 + PostGIS 3.7
缓存      Redis DB 8（未参与本次单节点读基准）
```

数据：

```text
vehicles      1,110,018
vehicle_state 1,110,018（基准租户全部 moving）
terminals     1,110,018（基准租户全部 online）
telemetry     1,259,854
数据库大小    2,836 MB
```

三个独立租户：

```text
bench-10K    10,000 台
bench-100K   100,000 台
bench-1000K  1,000,000 台
```

基准车辆状态为 `moving`，终端状态为 `online`，`online_rate=100%`、`terminal_online_rate=100%`。为避免离线清扫误伤，基准数据 heartbeat 设为远期时间；这仅用于压测，不代表生产心跳策略。

## 3. 测试方法

脚本：[scripts/bench_http.py](../scripts/bench_http.py)

每个规模：

1. 登录（Argon2 校验）
2. 等待 2.2s，测 dashboard 冷启动
3. dashboard 缓存命中 10 次
4. 车辆列表顺序 10 次
5. 地图顺序 10 次
6. bbox 顺序 5 次
7. nearby 顺序 5 次
8. 列表并发爆发、地图并发爆发
9. telemetry 写入 20 个点

并发与请求数：

| 规模 | 并发 | 爆发请求数 |
| --- | ---: | ---: |
| 10K | 20 | 100 |
| 100K | 10 | 50 |
| 1000K | 5 | 20 |

> 三档并发不同，是因为单机 4 vCPU 在 1M 全表统计下会被固定并发打满；三档结果用于容量边界，不是同并发吞吐对比。

## 4. 详细结果

### 4.1 10K 在线

```text
login                 683 ms
dashboard cold        212 ms
dashboard warm p50    8.9 ms
vehicles list seq     22.8 ms (p95 32.8)
list burst            185.9 ms p50 / 228.3 p95 / 241.7 p99
list burst RPS        100.7
map seq               38.9 ms
map burst             498.3 ms p50 / 697.2 p95
map burst RPS         39.1
bbox                  127.3 ms
nearby                139.8 ms
telemetry             87.1 ms p50 / 121.5 p95 / 95 RPS
```

### 4.2 100K 在线

```text
login                 615 ms
dashboard cold        1.57 s
dashboard warm p50    8.5 ms
vehicles list seq     39.7 ms (p95 46.5)
list burst            156.3 ms p50 / 224.8 p95 / 244.7 p99
list burst RPS        59.2
map seq               39.5 ms
map burst             177.4 ms p50 / 264.5 p95
map burst RPS         50.6
bbox                  121.2 ms
nearby                573.3 ms
telemetry             39.6 ms p50 / 57.6 p95 / 104 RPS
```

### 4.3 1000K 在线

```text
login                 605 ms
dashboard cold        3.62 s
dashboard warm p50    7.6 ms
vehicles list seq     281.4 ms (p95 342.5)
list burst            915.4 ms p50 / 2.12 s p95 / 2.23 s p99
list burst RPS        4.6
map seq               60.4 ms
map burst             166.7 ms p50 / 218.9 p95
map burst RPS         26.7
bbox                  119.1 ms
nearby                1.15 s
telemetry             20.2 ms p50 / 30.1 p95 / 91 RPS
```

## 5. 瓶颈分析

### 5.1 车辆列表（1M 主要瓶颈）

`list_vehicles` 需要：

```sql
SELECT COUNT(*) ...          -- 精确总数，扫描 1M
SELECT ... LIMIT 50 OFFSET 0 -- 再取一页
```

1M 下单请求 281ms，5 并发 p50 915ms、RPS 4.6。精确 `COUNT(*)` 是主要成本。

优化方向：

- 列表页使用 keyset pagination（`WHERE (org, plate) > (?, ?)`）替代 offset。
- 缓存精确总数（TTL 5–10s），或前端显示“约 N 台”。
- 租户规模 >100K 时使用预聚合 vehicle_stats 表。

### 5.2 dashboard 冷启动（1M 3.6s）

dashboard 需要精确统计全部 1M 车队/状态/终端。当前已把 5 个车队指标合并为一次扫描，但 1M 三表 join 仍需约 3.6s。

优化方向：

- 保持 2 秒缓存（当前命中 7.6ms）。
- 增加 dashboard_stats 预聚合表/物化视图，按组织定时刷新。
- 对 1M 租户允许“近似在线数”，用统计信息/采样。

### 5.3 nearby（1M 1.15s）

`nearby` 使用 `ST_DWithin(geography)` 后返回 500 条并排序。1M 点集中在约 0.5° 范围，5000m 半径命中量大，索引过滤后仍需距离计算。

优化方向：

- 增加结果距离上限和分页。
- 先 `ST_DWithin` 过滤 ID，再取前 N；避免对全部命中做最终排序。
- 对热点城市做网格/分区。

### 5.4 已修复的容量问题

本轮压测同时修复：

1. 车辆列表按状态 CASE 排序导致 1M 全排序 → 改为 `ORDER BY plate` + `(org_name, plate_no)` 索引。
2. 地图无上限 → `LIMIT 500`，避免 1M 行响应。
3. simulator 每 10 tick 更新全部 terminal 心跳 → 仅更新 demo 车辆。
4. 离线清扫 `status <> 'offline'` 无法用索引 → `status='online'`。
5. dashboard 11 个标量子查询 → 5 个车队指标一次扫描 + 告警/指令单独聚合。
6. 并发启动迁移 `schema_migrations` 主键冲突 → `ON CONFLICT DO NOTHING`。

### 5.5 启动/登录

login 约 0.6s，是 debug 构建 Argon2 成本；release 会明显降低。生产应使用 release。

## 6. 容量建议

| 规模 | 建议 |
| --- | --- |
| ≤10K | 单 API + PostgreSQL 足够；dashboard 缓存即可 |
| 100K | 单 API 可交互；建议列表总数缓存、dashboard 预聚合 |
| 1000K | 需要读写分离/多 API、列表 keyset、dashboard 预聚合、nearby 限流/分页；单机 4 vCPU 不适合作为 1M 在线生产配置 |

1M 在线不是当前单机配置的生产承诺；本次结果用于定位扩展边界和回归基线。

## 7. 复现

```powershell
# 1M 基准数据（10K/100K 同模板）
psql -h 127.0.0.1 -p 55432 -U tsp -d tsp -f scripts/bench_1000000.sql

# 单节点 API
$env:TSP_CLUSTER_MODE="false"
$env:TSP_DEMO_MODE="false"
cargo run

# HTTP 压测（串行执行，勿并行）
python scripts/bench_http.py 10K
python scripts/bench_http.py 100K
python scripts/bench_http.py 1000K
```

## 8. 回归验证

```text
cargo test       199 lib + 32 integration passed
cargo fmt --check   通过
cargo clippy --all-targets  无警告
```

最终 3 节点集群：8090/8091 Follower，8092 Leader，数据库连接正常。

原始结果：

- [10K 原始结果](bench-results/10k_online.json)
- [100K 原始结果](bench-results/100k_online.json)
- [1000K 原始结果](bench-results/1000k_online.json)
