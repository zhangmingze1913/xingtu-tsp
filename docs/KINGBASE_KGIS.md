# KingbaseES / KGIS（信创数据库）

本文说明如何用 KingbaseES + KGIS 替代 PostgreSQL + PostGIS，作为关系型与 GIS 数据存储。

## 1. 适用范围

- 信创环境要求国产数据库
- 已有 KingbaseES V8R6 / V9 实例
- 需要轨迹/车辆位置的 geometry、附近查询、bbox 查询
- 希望保留与 PostgreSQL/PostGIS 相同的业务 SQL 与 API

## 2. 连接方式

```bash
TSP_DATABASE='kingbase://tsp:password@kingbase:54321/tsp?sslmode=disable'
```

支持的 scheme：

```text
kingbase://
kingbase8://
kingbasees://
```

应用内部会规范化为 PostgreSQL wire URL，使用 SQLx PostgreSQL 驱动连接。KingbaseES 默认端口是 `54321`。

如果实例要求 TLS：

```bash
TSP_DATABASE='kingbase://tsp:password@kingbase:54321/tsp?sslmode=require'
```

## 3. KGIS 扩展

由 DBA 预先安装 KGIS 插件包，然后：

```sql
CREATE EXTENSION IF NOT EXISTS kingbase_spatial;
-- 或插件包使用 postgis 扩展名
CREATE EXTENSION IF NOT EXISTS postgis;
```

应用启动时：

1. 检查 `pg_extension` 中是否存在 `kingbase_spatial` 或 `postgis`
2. 不存在则尝试创建（需要权限）
3. 成功则：
   - 添加 `geometry(Point,4326)` 列
   - 创建 GiST 索引
   - 尝试创建 geography 表达式索引
   - 回填已有经纬度
   - 记录迁移 `0007_kgis_geometry`
4. 失败则 warning，并降级为纯 `lat/lon` 存储/查询，不阻塞启动

开关：

```bash
TSP_POSTGIS_ENABLED=true
```

## 4. 运行时行为

- `Db::engine_name()` 返回 `kingbase`
- `/api/health` 与 `/api/ready` 输出 `database_engine=kingbase`
- 空间查询仍走：
  - `ST_DWithin(geom::geography, ...)`
  - `ST_Intersects(geom, ST_MakeEnvelope(...))`
- KGIS 不提供 geography 索引时，仅保留 geometry 索引并记录 warning
- `pg_extension`/`pg_advisory_*` 不可用时降级，不阻塞启动
- Kingbase 下跳过 PostgreSQL 的 `INCLUDE` 覆盖索引，改用普通复合索引

## 5. 与 PostgreSQL 的差异

| 项目 | PostgreSQL/PostGIS | KingbaseES/KGIS |
| --- | --- | --- |
| 端口 | 5432 | 54321 |
| 驱动 | SQLx PostgreSQL | SQLx PostgreSQL wire |
| 空间扩展 | postgis | kingbase_spatial 或 postgis |
| 覆盖索引 | INCLUDE | 普通复合索引 |
| geography 索引 | 必须 | 尽力创建 |
| 管理锁 | pg_advisory_* | 尝试；失败时 warning 继续 |
| 实际验证 | 本机 PostgreSQL | 未连接真实 KES |

## 6. 集群与高可用

应用只配置一个 `TSP_DATABASE` URL。生产建议：

- KingbaseES 主备/读写分离由数据库侧提供
- 应用侧使用 VIP/负载均衡地址
- 不要依赖应用做数据库故障切换
- 集群模式仍需 Redis 与共享数据库

## 7. 从 PostgreSQL 迁移

1. 在 KingbaseES 创建库、用户、KGIS 扩展
2. 用 `pg_dump` / `ksql` 或厂商迁移工具导入 schema 与数据
3. 首次启动确认迁移版本与 `schema_migrations`
4. 执行：

```powershell
# 健康检查
Invoke-RestMethod http://127.0.0.1:8090/api/health

# 附近查询
Invoke-RestMethod "http://127.0.0.1:8090/api/vehicles/nearby?lat=31.23&lon=121.47&radius_m=5000"
```

5. 对比 PostgreSQL 与 Kingbase 的车辆数、轨迹条数、附近查询结果

## 8. 明确限制

- 本机没有 KingbaseES/KGIS 实例，未做真实连接验证
- 依赖 KingbaseES 对 PostgreSQL wire 协议的兼容性
- KGIS 函数兼容性以具体插件包为准；不保证所有 PostGIS 函数可用
- 未实现 Kingbase 专用驱动、批量 COPY、KGIS 特有索引类型
- 未在真实 KES 上验证 `pg_extension`、advisory lock、KGIS 函数兼容性
- 未验证 KES 主备切换、长事务、连接池上限

## 9. 自动验证脚本

```bash
ksql -h KES_HOST -p 54321 -U tsp -d tsp -f scripts/kingbase-verify.sql
```

应用侧真实连接测试：

```bash
TSP_TEST_KINGBASE_URL='kingbase://tsp:password@kes:54321/tsp?sslmode=disable' \
cargo test --lib kingbase_connection_smoke
```

## 10. 验证清单

```text
[ ] KingbaseES 可连接（54321）
[ ] CREATE EXTENSION kingbase_spatial/postgis 成功
[ ] geometry 列与 GiST 索引存在
[ ] telemetry/vehicle_state 写入 geom
[ ] /api/vehicles/nearby 返回正确
[ ] /api/vehicles/map bbox 返回正确
[ ] schema_migrations 含 0007_kgis_geometry
[ ] 与 PostgreSQL 结果抽样对比
```
