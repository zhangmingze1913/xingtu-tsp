# 性能评估（2026-11-25）

## 1. 结论摘要

本次评估在 **Windows 10 + Docker Desktop + KingbaseES V8R6 + 18 台演示车辆** 上完成，分别测了 debug 与 release，目标是验证当前版本在信创关系型数据库上的基本可用性与热点，不代表生产容量。

关键结果：

debug：

```text
车辆列表 50 条         p50 22.1 ms / p95 32.0 ms
车辆列表并发 10x5      50 请求 0.82 s，61.0 req/s，0 失败
dashboard 缓存命中     p50 3.9 ms
alerts 50 条           p50 23.0 ms
nearby（lat/lon 降级） p50 13.8 ms
trajectory 500 点      p50 31.9 ms / p95 185.9 ms
```

release（`cargo build --release`，同一 KES/18 车）：

```text
车辆列表 50 条         p50 12.9 ms / p95 25.9 ms
车辆列表并发 10x10     100 请求 0.56 s，178.1 req/s，0 失败
dashboard 缓存命中     p50 2.8 ms / p95 61.6 ms
alerts 50 条           p50 14.5 ms / p95 17.0 ms
nearby（lat/lon 降级） p50 7.9 ms / p95 11.0 ms
trajectory 500 点      p50 66.6 ms / p95 105.3 ms
```

结论：

- 小规模（18 车）下 API 与 KingbaseES 连接稳定。
- 车辆列表/告警的主要成本是数据库往返 + JSON 序列化。
- dashboard 冷缓存 p95 明显高于缓存命中，必须保留 2s TTL 或改为主动刷新。
- 500 点轨迹 p50 可接受，p95 受 JSON 序列化和 debug 构建影响。
- 本次 **未复测** 10K/100K/1M；历史报告见 [BENCHMARK_REPORT.md](BENCHMARK_REPORT.md)。

## 2. 环境

```text
OS        Windows 10 Pro 19045
CPU       4 vCPU
RAM       12 GB
构建       debug + release
API        单节点，TSP_CLUSTER_MODE=false
数据库     KingbaseES V008R006C008B0014（Docker，DB_MODE=pg）
缓存       Redis 未参与本组单机读基准
空间       KGIS 不可用，降级 lat/lon
车辆       18 台 demo
```

## 3. 方法

### 3.1 车辆列表

脚本：

```powershell
.\scripts\bench_api.ps1 -BaseUrl http://127.0.0.1:8096 `
  -Sequential 50 -Concurrency 10 -PerWorker 5
```

- 5 次预热
- 50 次顺序
- 10 并发 × 5 请求 = 50 请求
- 统计 avg/p50/p95/max、吞吐、失败数

结果：

```text
sequential n=50 avg=22.8ms p50=22.2ms p95=30.0ms max=40.8ms
concurrent n=50 workers=10 elapsed=0.82s throughput=61.0 req/s failed=0
```

### 3.2 release 端点延迟

| 端点 | n | avg | p50 | p95 |
| --- | ---: | ---: | ---: | ---: |
| `/api/vehicles?page_size=50` | 10 | 16.8 ms | 15.7 ms | 30.7 ms |
| `/api/dashboard` | 10 | 8.8 ms | 2.8 ms | 61.6 ms |
| `/api/alerts?page_size=50` | 10 | 13.3 ms | 14.5 ms | 17.0 ms |
| `/api/vehicles/nearby?...` | 10 | 8.1 ms | 7.9 ms | 11.0 ms |
| `/api/vehicles/{id}/trajectory?limit=500` | 10 | 67.7 ms | 66.6 ms | 105.3 ms |

### 3.3 debug 端点延迟

每端点 10 次（trajectory 5 次），同一 API/DB：

| 端点 | n | avg | p50 | p95 |
| --- | ---: | ---: | ---: | ---: |
| `/api/vehicles?page_size=50` | 10 | 21.3 ms | 22.1 ms | 32.0 ms |
| `/api/dashboard` | 10 | 24.7 ms | 3.9 ms | 210.0 ms |
| `/api/alerts?page_size=50` | 10 | 23.3 ms | 23.0 ms | 43.5 ms |
| `/api/vehicles/nearby?...` | 10 | 22.1 ms | 13.8 ms | 68.3 ms |
| `/api/vehicles/{id}/trajectory?limit=500` | 5 | 62.1 ms | 31.9 ms | 185.9 ms |

说明：

- dashboard 的 p50 是缓存命中；p95 包含缓存过期后的冷查询。
- nearby 在 KGIS 不可用时走 lat/lon + Haversine，不是空间索引结果。
- trajectory 500 点包含 DB 查询 + JSON 序列化。

## 4. 历史规模基准（非本次复测）

历史报告在 PostgreSQL 16.4 + PostGIS、debug 构建、单 API、4 vCPU/12GB 上测得：

| 规模 | dashboard 冷启动 | dashboard 缓存 | 列表 p50 | 地图 p50 | nearby p50 | 列表吞吐 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 10K | 212 ms | 8.9 ms | 22.8 ms | 38.9 ms | 140 ms | 101 RPS |
| 100K | 1.57 s | 8.5 ms | 39.7 ms | 39.5 ms | 573 ms | 59 RPS |
| 1000K | 3.62 s | 7.6 ms | 281 ms | 60.4 ms | 1.15 s | 4.6 RPS |

来源：[BENCHMARK_REPORT.md](BENCHMARK_REPORT.md)。这些数据是历史基线，不能直接等同于当前版本或生产容量。

## 5. 性能分析

### 5.1 车辆列表

热点：

1. `vehicles` + `vehicle_state` 查询
2. PII 批量解密（已从 N+1 改为批量）
3. JSON 序列化

优化建议：

- 列表页限制 `page_size <= 200`
- 大车队按组织/状态过滤
- 只返回列表必需字段
- 生产使用 release 构建

### 5.2 Dashboard

- 2 秒 TTL 缓存是主要保护
- 冷启动会执行多次聚合/趋势查询
- 高频刷新页面时不要绕过缓存

建议：

- 保持缓存
- 大租户拆分统计
- 将 7 日趋势改为预聚合表

### 5.3 轨迹

- 500 点 p95 185.9 ms 主要受 JSON 与 debug 影响
- 生产应按时间范围 + `limit` 查询，不要一次拉全量
- 10 万级以上建议 TDengine 或预聚合

### 5.4 信创数据库

- KingbaseES debug 列表 61 RPS 是单机小数据结果
- 没有 KGIS 时 nearby 走内存过滤，车辆数大时退化明显
- 需要 KGIS 插件后复测空间查询
- `level` 引号化仅在 Kingbase 路径启用，对 PG/MySQL/SQLite 无影响

## 6. 代码结构优化

本次把 `lib.rs` 中约 450 行后台任务迁到 `services/maintenance.rs`：

```text
lib.rs          696 -> 244 行
maintenance.rs  461 行
```

同时把 `api/integrations.rs` 拆为：

```text
api/integrations/mod.rs       对账、历史、CSV
api/integrations/export.rs    导出任务、令牌、失败告警
```

收益：

- 入口文件只保留装配/生命周期
- 后台任务集中，便于单独测试和排障
- 对账与导出边界清晰
- 降低合并冲突

对运行时性能无直接影响，但降低维护成本。

## 7. 建议

1. 用 release 构建复测上述端点。
2. 在 PostgreSQL/Kingbase 上执行 `EXPLAIN (ANALYZE, BUFFERS)`。
3. 对 `vehicles/telemetry/alerts` 做 10K/100K/1M 复测。
4. KGIS 可用后单独测 `ST_DWithin`/`ST_Intersects`。
5. 高并发下调整连接池，不要盲目增大。
6. 大导出使用后台任务 + 分页，不要同步全量。

## 8. 限制

- release 数据仍为 18 车、单机、无 KGIS，不是生产容量
- 18 台 demo 车辆，不是规模数据
- KGIS 不可用
- 未测 Redis/Raft 集群开销
- 未测真实终端/弱网
- 未测百万级当前版本

> 本报告只证明“当前环境可运行、基本延迟可接受”，不构成生产容量承诺。


## 9. PostGIS 空间规模复测

环境：`postgis/postgis:16-3.4`，PostgreSQL 16.16，4 vCPU/12 GB。

数据：

```text
100 万行，geom 1000000/1000000
总大小 578 MB
heap 174 MB
indexes 404 MB
```

`count(*)` 查询：

| 规模 | 5 km nearby | 0.15° bbox |
| --- | ---: | ---: |
| 10 万 | 938 ms | 48 ms |
| 100 万 | 14.6 s | 348 ms |

计划：

```text
nearby: Bitmap Index Scan on idx_telemetry_geog
bbox:   Bitmap Index Scan on idx_telemetry_geom
```

结论：

- bbox 在 100 万级仍为百毫秒级，GiST 有效。
- nearby 的 geography 距离计算在 100 万级达到 14.6s；必须加时间窗/分页/分区，或把高频轨迹交给 TDengine。
- 这是全量 count，不是分页 API 延迟。


## 10. 2026-11-26 dashboard 查询优化

热点：7 天趋势原本每天 2 条 SQL，共 14 次数据库往返。

优化：`src/db/dashboard.rs` 改为两条聚合 SQL，每条用 7 个 `CASE` 范围精确覆盖中国日边界。

同一轮还把车辆汇总和状态分布从 2 次扫描合并为 1 次分组扫描（`fleet_status_future`），dashboard 冷路径从 5 条查询降为 4 条。

```text
原：14 次往返
新：2 次往返（mileage + alerts）
```

### release 复测

环境：KingbaseES V8R6，18 台 demo 车辆，release 构建。

```text
车辆列表 100 次：avg 12.9–18.1 ms，p50 9.8–11.4 ms，p95 22.2–31.6 ms
并发 10x10 热测：100 请求 0.18–0.32 s，281.7–313.4 req/s，0 失败
dashboard 预热后冷查询：18–159 ms
dashboard 首次冷启动：1.2–2.8 s（KES/进程冷）
dashboard 热：p50 1.6–4.6 ms
```

首轮并发曾出现 6.4 req/s、1 失败、15.66 s 的异常；复测为 281.7–313.4 req/s、0 失败。异常与 KES/Docker 冷 IO 有关，未从报告中删除。

说明：

- 并发首轮曾出现 16.2 req/s 的冷启动异常，预热后复测为 313.4 req/s；报告保留热测结果并标注波动。
- dashboard 优化主要减少数据库往返；冷启动仍受 DB/缓存影响。
- 该结果为 18 车小数据，不是生产容量承诺。

### 10.1 冷缓存 single-flight

`DashboardCache` 增加按组织 single-flight：

- 缓存未命中时只允许一个请求执行聚合
- 其余同组织请求等待并复用结果
- 不同组织互不阻塞
- TTL 仍为 2s

10 并发冷缓存 burst 实测：

```text
10/10 成功
总耗时：33.5 / 45.2 / 48.3 / 268.4 / 512.3 ms
```

这是 18 车、KES、同组织 burst 的结果；没有做“无 single-flight”严格 A/B，不能量化成固定百分比。


## 11. 2026-11-27 PII hydration 分配优化

`get_pii_batch` 从 `HashMap<(String, String), String>` 改为按 owner 的嵌套 map：

```text
HashMap<owner_id, HashMap<field, ciphertext>>
```

调用方用 `&str` 查找，去掉每行：

```text
vehicle.id.clone()
"driver_phone".to_string()
"driver_name".to_string()
```

50 行页面由约 150 次小分配降为 0 次查找分配。该改动不改变查询次数或协议语义。


## 12. 2026-11-27 整体性能与火焰图

### 工具

```text
WPT/xperf  已从 ADK ISO 安装
samply     已实际采样
```

profile：

```text
D:\tsp-profile-sqlite.json.gz
D:\tsp-profile-debug.json.gz
D:\tsp-profile-sym2.json.gz
D:\tsp-profile-sym3.json.gz
```

### SQLite 负载

```text
release 10x20：824.9 req/s，0 失败
debug   10x20：225.3 req/s，0 失败
```

### 已做优化

```text
dashboard 趋势 14 -> 2 SQL
车辆汇总/状态 3 -> 2 查询
dashboard per-org single-flight
PII hydration 改为 owner 嵌套 map，去掉逐行 id/field 分配
JT1078 gap-skip 死循环修复
畸形输入 fuzz 1000 组/7 入口
```

### 火焰图限制

samply profile 的 `meta.symbolicated=false`，栈仍是地址：

```text
0xa0fd4
0xa03d4
```

已尝试 release/debug + PDB + `--unstable-presymbolicate`，仍未得到函数名。
因此不把地址栈当作火焰图结论，也不伪造 SVG。

### 潜在热点

```text
dashboard 冷查询仍受 DB IO 影响
telemetry 批量仍是逐点事务内查询
JT808 每帧分配
PII 解密/审计
导出 CSV/加密
```

这些需要用可符号化 flamegraph 或数据库慢日志进一步确认。


## 13. 2026-11-27 0x0704 批量预取

原 `ingest_locations` 对批次内每个点执行一次：

```text
SELECT vehicle + vehicle_state
```

现改为每个不同 vehicle 预取一次：

```text
一次 SELECT -> VehicleContext
逐点在内存推进 previous state
```

保留：

```text
同一事务
时间排序
claim 去重
trip/fence/alert 业务
```

批量 100 点从约 100 次车辆/状态查询降为 1 次（单车辆批次）。
测试：`tests/storage.rs`、`tests/jt808_terminal_sim.rs`。


## 14. 2026-11-27 JT808 连接 buffer 上限

`handle_connection` 原逻辑在收到 `0x7e` 后等待闭合；若对端只发 `0x7e` + 无限数据，`buffer` 可无界增长。

新增：

```text
MAX_JT808_BUFFER_BYTES = 64 KiB
buffer.len() > limit -> warn + close connection
```

这是性能/内存 DoS 防护，不改变合法帧解析。

同时修正 `registration.rs` 缩进，`raft_log` 使用 `io::Error::other` 满足 clippy。


## 15. 2026-11-28 Docker/K8s 实测

```text
Docker SQLite：10x20 222.3 req/s，0 失败
K8s kind SQLite：10x10 159.8 req/s，0 失败
```

容器启动发现 SQLite backfill 的 UPSERT 语法错误，已修复：

```text
SQLite: INSERT OR IGNORE
Postgres: ON CONFLICT DO NOTHING
MySQL: INSERT IGNORE
```

同时修复 JT808 无界 buffer（64 KiB 上限）和 registration 缩进。


## 16. 2026-11-28 全链路压测（Docker/K8s）

脚本：

```powershell
.\scripts\fullchain-smoke.ps1 `
  -BaseUrl http://127.0.0.1:18090 `
  -Jt808Addr 127.0.0.1:18091
```

同一轮同时执行：

```text
HTTP 登录/车辆列表/dashboard/并发基准
JT808 模拟终端：鉴权 + 5 次位置上报
```

### Docker SQLite

```text
HTTP 10x20：279.2–337.1 req/s，0 失败
dashboard：5.5–15.3 ms
JT808：鉴权 + 5 点全部 result=0
容器：CPU 1.19%，内存 105.4 MiB
```

### kind SQLite

```text
HTTP 10x10：153.3 req/s，0 失败
JT808：鉴权 + 5 点全部 result=0
```

结论：

```text
功能链路通过
容器 CPU 很低，瓶颈在客户端/端口转发/SQLite 单写
不是生产容量结论
```


## 17. Rust 1.100 工具链

```text
rustc 1.100.0
cargo 1.100.0
```

Docker builder 改为：

```text
rust:1.100-bookworm
```

Cargo.toml 声明：

```toml
rust-version = "1.100"
```


### 1.100 Docker 限制

宿主 1.100 测试通过；Docker `rust:1.100-bookworm` 构建在应用 crate 链接阶段长时间无输出，未完成。
本轮 Docker/K8s 结果来自 `rust:1.94.1` 镜像，不能写成 1.100 镜像结果。


## 18. 百万连接前置改造

本轮把两个明确风险改为：

```text
每连接下行：mpsc::channel(32) + try_send
session：64-way shard，每 shard 独立 Mutex
```

测试：

```text
bounded_outbox_rejects_when_full
session_new_cancels_old...
```

仍未做 1M 实测；32 队列容量和 64 shard 需要真实压测确认。
