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
