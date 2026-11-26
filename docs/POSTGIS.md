# PostGIS 空间数据库

## 1. 启用方式

要求 PostgreSQL 已安装 PostGIS 扩展，且数据库用户有权执行 `CREATE EXTENSION`（或由 DBA 预先创建）。

```sql
CREATE EXTENSION IF NOT EXISTS postgis;
```

应用启动时会：

1. 检测 `pg_extension` 中是否存在 `postgis`；
2. 不存在则尝试 `CREATE EXTENSION IF NOT EXISTS postgis`；
3. 成功则：
   - 给 `telemetry`、`vehicle_state` 增加 `geometry(Point,4326)`；
   - 回填已有经纬度数据；
   - 创建 GiST 空间索引（geometry 与 geography 表达式）；
   - 记录迁移 `0007_postgis_geometry`；
4. 失败则记录 warning，并**降级为纯 lat/lon 存储/查询**，不影响启动。

环境开关：

```bash
TSP_POSTGIS_ENABLED=true   # 默认 true；SQLite/MySQL 自动忽略
```

## 2. 数据结构

```sql
ALTER TABLE telemetry ADD COLUMN geom geometry(Point,4326);
ALTER TABLE vehicle_state ADD COLUMN geom geometry(Point,4326);

CREATE INDEX idx_telemetry_geom ON telemetry USING GIST (geom);
CREATE INDEX idx_vehicle_state_geom ON vehicle_state USING GIST (geom);
CREATE INDEX idx_telemetry_geog ON telemetry USING GIST ((geom::geography));
CREATE INDEX idx_vehicle_state_geog ON vehicle_state USING GIST ((geom::geography));
```

写入时同时保留 `lat`/`lon` 列，保证 SQLite/MySQL 和旧客户端兼容：

- `telemetry.geom = ST_SetSRID(ST_MakePoint(lon, lat), 4326)`
- `vehicle_state.geom` 同步更新

## 3. API

### 附近车辆

```http
GET /api/vehicles/nearby?lat=31.23&lon=121.47&radius_m=5000
```

PostGIS：`ST_DWithin(geom::geography, point::geography, radius_m)`。
非 PostGIS：读取当前车辆位置后用 Haversine 过滤。

### 地图 bbox

```http
GET /api/vehicles/map?bbox=121.2,31.0,121.8,31.4
```

PostGIS：`ST_Intersects(geom, ST_MakeEnvelope(min_lon,min_lat,max_lon,max_lat,4326))`。
非 PostGIS：lat/lon 边界过滤。

两个接口都继续受 `org_name` 组织域约束。

## 4. 本机验证

本机便携 PostgreSQL 16.4 已安装 PostGIS 3.7.0：

- `CREATE EXTENSION postgis` 成功
- `telemetry.geom` 回填 43099 行
- `vehicle_state.geom` 18 行
- 新位置写入后 `ST_AsText(geom)` 为 `POINT(121.4737 31.2304)`
- `/api/vehicles/map?bbox=...` 返回 9 台
- `/api/vehicles/nearby?lat=31.23&lon=121.47&radius_m=5000` 返回 1 台
- 小数据量下规划器仍可能选 Seq Scan；`SET enable_seqscan=off` 时确认使用 `idx_vehicle_state_geog`

## 5. 注意

- PostGIS 是可选能力；没有扩展时应用仍可运行，但不能使用数据库侧空间索引。
- 空间查询结果仍受 500 条上限约束。
- 生产建议：
  - PostGIS 扩展由 DBA 预创建；
  - `ANALYZE` 后观察 `EXPLAIN (ANALYZE, BUFFERS)`；
  - 轨迹大表按时间分区 + GiST 索引；
  - 高频轨迹可继续落 tsink/TDengine，PostGIS 保留最近 N 天空间索引。


---

## KingbaseES / KGIS

信创环境可使用 KingbaseES + KGIS 替代 PostgreSQL + PostGIS，连接串与限制见 [KINGBASE_KGIS.md](KINGBASE_KGIS.md)。


## 6. 10 万/100 万级空间基准（2026-11-25）

容器：

```text
postgis/postgis:16-3.4
PostgreSQL 16.16
PostGIS 3.4
```

脚本：

```bash
psql -U tsp -d tsp -v rows=100000 -f scripts/postgis-bench.sql
psql -U tsp -d tsp -v rows=1000000 -f scripts/postgis-bench.sql
```

数据：

```text
100 万行：geom 1000000/1000000
总大小 578 MB
heap 174 MB
indexes 404 MB
```

查询（`count(*)`，5 km 半径 / 约 0.15° bbox）：

| 规模 | nearby | bbox |
| --- | ---: | ---: |
| 10 万 | 938 ms | 48 ms |
| 100 万 | 14.6 s | 348 ms |

计划：

```text
nearby: Bitmap Index Scan on idx_telemetry_geog
bbox:   Bitmap Index Scan on idx_telemetry_geom
```

说明：

- 这是全量 `count(*)`，不是分页 API；分页应加 LIMIT 并避免全量聚合。
- 100 万级 nearby 14.6s 表明 geography 距离过滤仍重；生产应按时间分区/缩小时间窗，或把高频轨迹交给 TDengine/tsink。
- bbox 348ms 说明 GiST 几何索引可用。
- 结果来自 4 vCPU / 12 GB Docker Desktop，不是生产容量承诺。


## 健康接口 PostGIS 分支

可选测试（需要一次性 PostGIS 数据库）：

```powershell
$env:TSP_TEST_POSTGIS_URL='postgres://tsp:tsp-dev-password@127.0.0.1:55433/tsp?sslmode=disable'
cargo test --test postgis_health
```

验证：

```text
/api/health database_engine=postgres
/api/health spatial_backend=postgis
/api/ready spatial_backend=postgis
Db::vehicles_in_bbox -> 命中测试车辆
Db::vehicles_nearby -> 命中测试车辆
```

这是 PostgreSQL/PostGIS 兼容路径验证，不是 KGIS 插件/License 验证。
