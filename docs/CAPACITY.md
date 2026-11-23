# 10 万级 Telemetry 容量测试

测试库：PostgreSQL 16.4 + PostGIS 3.7，单表 `telemetry`。
数据量：

```text
总行数       约 149,000（含运行中模拟器新增）
veh_001      约 108,000 行（容量测试新增 100,000 行）
```

生成脚本见 `scripts/capacity.sql`（仅压测环境使用）。

## 1. 轨迹查询

查询形态：

```sql
SELECT ... FROM telemetry
WHERE vehicle_id = 'veh_001'
  AND ts >= now() - interval '7 days'
  AND ts <= now()
ORDER BY ts DESC
LIMIT 1000;
```

`VACUUM (ANALYZE) telemetry` 后：

```text
Limit
  -> Index Scan Backward using idx_telemetry_ts
       Filter: vehicle_id = 'veh_001'
Execution Time: 2.868 ms
Buffers: shared hit=42
```

优化前同查询未 VACUUM/未稳定统计时：

```text
Seq Scan on telemetry
Execution Time: 363.248 ms
Buffers: shared hit=3035
```

结论：

- 7 天范围覆盖 108k 行时，规划器最初倾向 Seq Scan。
- `VACUUM (ANALYZE)` 后选择 `idx_telemetry_ts` 反向扫描并提前 LIMIT，降到 **2.9ms**。
- 新增 PostgreSQL 覆盖索引：
  ```sql
  CREATE INDEX idx_telemetry_vehicle_ts_cover
  ON telemetry (vehicle_id, ts DESC)
  INCLUDE (lat, lon, speed, heading, ignition, soc, odometer_km, source);
  ```
  供数据/查询形态变化时使用；是否命中以 `EXPLAIN` 为准。

## 2. API 实测（debug）

```text
GET /api/vehicles/veh_001/trajectory?...&limit=1000
10 次：90–168ms，1000 个点
```

DB 查询约 2.9ms，其余主要是 debug 构建下的 JSON 序列化和 1000 点响应体。
release 构建会明显降低。

## 3. Dashboard

```text
冷启动（缓存过期）：73.9ms
缓存命中：3.9ms
```

dashboard 标量统计已合并为 **一次多子查询 SELECT**，状态分布与标量统计通过 `tokio::try_join!` 并发执行。

## 4. 空间查询

18 台车规模，`vehicle_state` 仅 18 行：

```text
SET enable_seqscan=off;
Index Scan using idx_vehicle_state_geog
Execution Time: 15.7ms
```

小表规划器默认 Seq Scan 是合理的；数据增长后 geography GiST 索引生效。

## 5. 慢查询采样

PostgreSQL 侧建议：

```sql
CREATE EXTENSION IF NOT EXISTS pg_stat_statements;

-- postgresql.conf
shared_preload_libraries = 'pg_stat_statements'
pg_stat_statements.track = all
log_min_duration_statement = 100ms
```

采样 SQL：

```sql
SELECT calls, round(mean_exec_time::numeric, 2) AS avg_ms,
       round(max_exec_time::numeric, 2) AS max_ms, rows, query
FROM pg_stat_statements
WHERE query ILIKE '%telemetry%'
ORDER BY mean_exec_time DESC
LIMIT 20;
```

本机已启用 `pg_stat_statements` 并实际采样（`pg_stat_statements_reset()` 后执行 3 次轨迹/看板）：

```text
calls | avg_ms | max_ms | rows | query
6     | 23.25  | 99.85  | 0    | UPDATE telemetry SET geom = ...（启动回填，一次性）
1     | 4.21   | 4.21   | 1    | dashboard 多子查询 SELECT
3     | 3.91   | 4.70   | 3000 | trajectory SELECT
4     | 3.33   | 10.17  | 4    | INSERT INTO telemetry
```

同时用 `EXPLAIN (ANALYZE, BUFFERS)` 确认：

- 轨迹：`idx_telemetry_ts` / 覆盖索引；
- 附近：geography GiST；
- dashboard：单次聚合查询。

## 6. 容量建议

1. `telemetry` 按 `ts` 月分区，保留策略按分区 DROP，避免大表 DELETE。
2. 高频轨迹冷热分层：PostGIS 保留近 N 天，历史进 tsink/TDengine。
3. 定时 `ANALYZE`，批量导入后 `VACUUM (ANALYZE)`。
4. 轨迹接口限制 `limit <= 2000`；大范围查询用 bbox/时间窗。
5. 生产开启 `pg_stat_statements` + `log_min_duration_statement`。
