# 时序后端选择：PostgreSQL / TDengine

不同体量的使用者可以选择不同的轨迹存储后端，而不改变 API 与前端。

## 1. 选择建议

| 体量 | 推荐后端 | 理由 |
| --- | --- | --- |
| 开发 / 单机 / ≤1 万在线 | `postgres`（或 SQLite 单机） | 零额外组件，部署简单 |
| 1 万–10 万在线 | `postgres` + PostGIS | 事务、权限、空间查询成熟；当前默认 |
| 10 万+ 在线 / 长周期轨迹分析 | `tdengine` | 列式时序压缩、按时间窗口聚合更省成本 |
| 从 PG 迁移到 TDengine | `dual` | 双写观察，查询优先 TDengine，失败回退 PG |

TDengine 是**查询/分析加速层**，PostgreSQL 仍是车辆、用户、告警、指令和最新状态的权威库。

## 2. 三种模式

### `postgres`

```text
TSP_TIMESERIES_BACKEND=postgres
```

- 不创建 TDengine 客户端
- 轨迹查询只走 PostgreSQL
- 即使设置了 `TSP_TDENGINE_URL` 也会忽略并记录 info 日志

### `tdengine`

```text
TSP_TIMESERIES_BACKEND=tdengine
TSP_TDENGINE_URL=http://tdengine:6041
```

- 启动时必须成功创建数据库/超级表，否则应用启动失败
- 轨迹查询优先 TDengine，空结果或错误回退 PostgreSQL
- PostgreSQL 仍持续写入，作为权威数据与回退源

### `dual`

```text
TSP_TIMESERIES_BACKEND=dual
TSP_TDENGINE_URL=http://tdengine:6041
```

- 同时写 PostgreSQL 与 TDengine
- 查询顺序：tsink（如启用）→ TDengine → PostgreSQL
- TDengine 初始化失败时降级为 PG-only 并告警
- 适合迁移期对比和灰度

未显式设置 `TSP_TIMESERIES_BACKEND` 时：

```text
有 TSP_TDENGINE_URL -> dual
无 TSP_TDENGINE_URL -> postgres
```

## 3. 配置

```bash
TSP_TIMESERIES_BACKEND=postgres   # postgres | tdengine | dual
TSP_TDENGINE_URL=http://tdengine:6041
TSP_TDENGINE_USER=root
TSP_TDENGINE_PASSWORD=taosdata
TSP_TDENGINE_DATABASE=tsp
TSP_TDENGINE_QUEUE_CAPACITY=8192
TSP_TDENGINE_WRITE_RETRIES=3
TSP_TDENGINE_SPOOL_DIR=data/tdengine-spool
TSP_TDENGINE_SPOOL_MAX_FILES=100000
TSP_TDENGINE_SPOOL_REPLAY_SECS=60
TSP_TIMESERIES_RECONCILE_ENABLED=true
TSP_TIMESERIES_RECONCILE_SECS=300
TSP_TIMESERIES_RECONCILE_WINDOW_MINUTES=15
TSP_TIMESERIES_RECONCILE_MAX_VEHICLES=100
```

`TSP_TDENGINE_URL` 必须是 taosAdapter 的 HTTP 地址，不是原生 6030 端口。

健康检查：

```bash
curl http://127.0.0.1:8090/api/health
```

返回：

```json
{
  "database": true,
  "timeseries_backend": "tdengine",
  "tdengine": true,
  "tdengine_ready": true
}
```

`tdengine=false` 表示未启用或初始化失败；`tdengine_ready` 是实时 `SELECT SERVER_VERSION()` 探测结果。

当 `TSP_TIMESERIES_BACKEND=tdengine` 时，`/api/ready` 会要求 TDengine 探测成功；`dual` 模式不阻塞就绪。

## 4. Docker Compose

TDengine 使用 Compose profile，默认不启动：

```bash
TSP_TIMESERIES_BACKEND=tdengine \
TSP_TDENGINE_URL=http://tdengine:6041 \
docker compose --profile tdengine up -d tdengine tsp
```

首次启动顺序建议：

```bash
docker compose --profile tdengine up -d tdengine
# 等 TDengine 健康
docker compose up -d tsp
```

镜像：`tdengine/tdengine:3.3.6.0`，REST 端口 `6041`，原生端口 `6030`。

## 5. Kubernetes

```bash
kubectl -n xingtu-tsp apply -f deploy/k8s/tdengine.yaml
kubectl -n xingtu-tsp edit configmap tsp-config
```

修改：

```yaml
TSP_TIMESERIES_BACKEND: "tdengine"
TSP_TDENGINE_URL: "http://tdengine:6041"
```

然后滚动重启 API 与 gateway：

```bash
kubectl -n xingtu-tsp rollout restart deployment/tsp-api
kubectl -n xingtu-tsp rollout restart deployment/tsp-jt808
```

`deploy/k8s/tdengine.yaml` 是单节点评估清单，未加入默认 Kustomization；生产使用 TDengine Operator/Helm 三节点集群。

### 持久化 spool

默认 K8s spool 位于 `/tmp/tdengine-spool`，Pod 重建会丢失。若 StorageClass 支持 `ReadWriteMany`：

```bash
kubectl -n xingtu-tsp apply -f deploy/k8s/tdengine-spool.yaml
```

给 API/网关挂载：

```bash
kubectl -n xingtu-tsp patch deployment tsp-api --type=strategic -p '
{"spec":{"template":{"spec":{
  "volumes":[{"name":"tdengine-spool","persistentVolumeClaim":{"claimName":"tdengine-spool"}}],
  "containers":[{"name":"api","volumeMounts":[{"name":"tdengine-spool","mountPath":"/tmp/tdengine-spool"}]}]
}}}}'

kubectl -n xingtu-tsp patch deployment tsp-jt808 --type=strategic -p '
{"spec":{"template":{"spec":{
  "volumes":[{"name":"tdengine-spool","persistentVolumeClaim":{"claimName":"tdengine-spool"}}],
  "containers":[{"name":"gateway","volumeMounts":[{"name":"tdengine-spool","mountPath":"/tmp/tdengine-spool"}]}]
}}}}'
```

如果只有 `ReadWriteOnce`，不要多副本共享同一 PVC；应改为每 Pod 独立 StatefulSet PVC，或接受临时 spool。

Compose 集群已使用命名卷 `tdengine-spool`，可直接持久化。

## 6. 数据模型

应用启动时自动执行：

```sql
CREATE DATABASE IF NOT EXISTS `tsp`;
CREATE STABLE IF NOT EXISTS `tsp`.`telemetry` (
  ts TIMESTAMP, lat DOUBLE, lon DOUBLE, speed DOUBLE, heading DOUBLE,
  ignition BOOL, soc DOUBLE, odometer DOUBLE
) TAGS (vehicle_id NCHAR(64));
```

每个车辆一个子表：

```text
v_<sanitized_vehicle_id>
```

查询使用标签过滤 + 时间范围 + LIMIT，避免全表扫描。

## 7. 写入可靠性与指标

- 有界队列：`TSP_TDENGINE_QUEUE_CAPACITY`（默认 8192）
- 队列满：丢弃最新样本并告警，不阻塞 1078 接收
- 单条写入失败：最多重试 `TSP_TDENGINE_WRITE_RETRIES` 次，指数退避
- 重试耗尽：写入磁盘 spool，后台重放
- spool 每个样本一个 JSON 文件，成功后删除
- `TSP_TDENGINE_SPOOL_MAX_FILES` 限制积压文件数

- 指标：

```text
xingtu_tdengine_writes_total
xingtu_tdengine_failures_total
xingtu_tdengine_retries_total
xingtu_tdengine_dropped_total
xingtu_tdengine_queue_depth
xingtu_tdengine_spooled_total
xingtu_tdengine_spool_replayed_total
xingtu_tdengine_spool_pending
xingtu_tdengine_spool_failed_total
```

## 8. 双写对账

管理员接口：

```http
GET /api/integrations/timeseries/reconcile?vehicle_id=<id>&from=<rfc3339>&to=<rfc3339>
Authorization: Bearer <admin-token>
```

返回：

```json
{
  "postgres": {"count": 120, "min_ts": "...", "max_ts": "..."},
  "tdengine": {"count": 118, "min_ts": "...", "max_ts": "..."},
  "count_delta": 2,
  "consistent": false
}
```

- 默认时间窗口：最近 60 分钟
- `from/to` 必须是 RFC3339
- 仅 admin 可调用
- 这是计数/边界对账，不是逐点哈希
- 指标：`xingtu_tdengine_reconcile_checks_total`、`xingtu_tdengine_reconcile_mismatches_total`

定时对账：

- 默认启用（`tdengine`/`dual`）
- 仅集群 leader 执行
- 每 5 分钟检查最近 15 分钟活跃车辆（最多 100 台）
- 不一致写 warn 日志并累加 mismatch 指标

## 9. 一致性与限制

当前实现：

- PostgreSQL 先写，TDengine 异步队列写
- TDengine 写失败只记录 warn，不阻塞主链路
- 没有 outbox、对账或重放
- TDengine 与 PG 可能存在秒级/分钟级差异
- 查询回退 PG 可能掩盖 TDengine 缺数据

因此：

```text
PG = 权威
TDengine = 可重建的查询加速副本
```

生产启用前应补：

- 写入失败持久化队列（可复用 RustFS spool 思路）
- 行数/时间范围对账
- TDengine 写入延迟与失败指标
- 保留策略与磁盘容量规划

## 10. 真实 TDengine 联调脚本

```powershell
.\scripts\tdengine-smoke.ps1
```

执行：

- `SELECT SERVER_VERSION()` 连通性检查
- 创建数据库/超级表
- 插入一条样本
- 按车辆标签查询 COUNT

参数：

```powershell
.\scripts\tdengine-smoke.ps1 -Url http://127.0.0.1:6041 -Database tsp
```

## 11. 容量/压测脚本

```powershell
.\scripts\tdengine-bench.ps1
```

可选参数：

```powershell
.\scripts\tdengine-bench.ps1 -Vehicles 100 -SamplesPerVehicle 600 -BatchSize 100
.\scripts\tdengine-bench.ps1 -Cleanup
```

输出：

- 写入行数与 rows/s
- 全表 COUNT 延迟
- 单车辆范围查询延迟
- 可选删除压测库

注意：这是 bounded smoke benchmark，不是生产容量认证；结果取决于机型、磁盘、网络和 TDengine 版本。

## 12. 测试

代码侧测试：

```text
tdengine::tests::parses_timestamps_and_identifiers
tdengine::tests::rest_query_maps_rows_to_points
tdengine::tests::write_retries_transient_failures
tdengine::tests::range_stats_maps_count_and_bounds
tdengine_spool::tests::spool_save_pending_remove_roundtrip
tdengine_spool::tests::spool_enforces_max_files
config::tests::parses_time_series_backends
```

REST mock 验证：

- `POST /rest/sql`
- CREATE DATABASE/STABLE
- SELECT 行到 `TelemetryPoint` 映射

未验证：

- 真实 TDengine 3.x 容器
- 真实 10 万/百万级轨迹查询
- 双写延迟与不一致窗口
