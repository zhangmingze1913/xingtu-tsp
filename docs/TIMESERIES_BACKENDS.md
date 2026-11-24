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
TSP_TIMESERIES_RECONCILE_HISTORY_DAYS=90
TSP_RECONCILE_ALERT_WEBHOOK=https://alerts.example/hooks/tdengine
TSP_RECONCILE_ALERT_COOLDOWN_SECS=300
TSP_RECONCILE_ALERT_COOLDOWN_SCOPE=global
TSP_RECONCILE_ALERT_RETRIES=2
TSP_RECONCILE_ALERT_FORMAT=alertmanager
TSP_RECONCILE_ALERT_MODE=summary
TSP_RECONCILE_ALERT_WEBHOOK_SECRET=change-me
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

导出目录持久化：

```bash
kubectl -n xingtu-tsp apply -f deploy/k8s/reconcile-export-pvc.yaml
```

```bash
kubectl -n xingtu-tsp patch deployment tsp-api --type=strategic -p '
{"spec":{"template":{"spec":{
  "volumes":[{"name":"reconcile-export","persistentVolumeClaim":{"claimName":"reconcile-export"}}],
  "containers":[{"name":"api","volumeMounts":[{"name":"reconcile-export","mountPath":"/tmp/reconcile-exports"}]}]
}}}}'
```

同样要求 `ReadWriteMany`；否则固定单副本执行导出。

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
- 结果写入 `timeseries_reconciliations` 表
- 历史查询：

```http
GET /api/integrations/timeseries/reconciliations?limit=50
```

- `limit` 1–200，默认 50
- admin 可查全部组织；非管理员只能查自己 `org_name`
- 未绑定组织的非管理员返回 403；请求里的 `org_name` 会被忽略
- 可选 `vehicle_id`、`org_name`、`consistent=true|false` 过滤
- keyset 游标：响应含 `next_cursor`（`checked_at|id`），下一页传 `cursor=<next_cursor>`
- 历史默认保留 90 天，每 6 小时清理；`0` 关闭
- CSV 导出：

```http
GET /api/integrations/timeseries/reconciliations/export?vehicle_id=...&org_name=...&consistent=false
```

- 复用 `limit`（最大 5000，默认 2000）、`vehicle_id`、`org_name`、`consistent` 过滤
- 行数据按 500 行/页 keyset 查询并流式输出，避免全量加载
- 分页下载协议：

```http
GET /api/integrations/timeseries/reconciliations/export/page?limit=500&cursor=<checked_at>|<id>
```

- 响应头：`X-Row-Count`、`X-Next-Cursor`（仅满页返回）
- 非法 `cursor` 返回 400
- CSV 行仍做字段转义
- CSV 字段做引号转义
- `footer=true` 时末尾输出 `#END,<rows>`；缺失该行表示传输/查询中断
- 默认不输出 footer，保持标准 CSV 兼容
- CSV 含 `org_name` 列
- `Content-Type: text/csv`
- 定时任务只持久化 mismatch，手动接口每次持久化
- mismatch 可推送 JSON webhook；全局冷却默认 300s
- 失败重试 2 次（`TSP_RECONCILE_ALERT_RETRIES`）
- 配置密钥后发送 `X-TSP-Signature: sha256=<hex>`（HMAC-SHA256）
- `TSP_RECONCILE_ALERT_FORMAT=generic|alertmanager`
- Alertmanager 格式为 v4 JSON 数组，labels 含 `alertname/severity/service/vehicle_id`
- `TSP_RECONCILE_ALERT_MODE=per_vehicle|summary`
  - `per_vehicle`：每车一条（受冷却约束）
  - `summary`：每次定时对账聚合一条
- 历史清理指标：`xingtu_tdengine_history_pruned_total`
- 指标：`xingtu_tdengine_alert_sent_total`、`failed_total`、`skipped_total`
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

### spool 运维接口

```http
GET /api/integrations/timeseries/spool
POST /api/integrations/timeseries/spool/replay
```

- admin only
- GET 返回 `{files, bytes}`
- POST 立即重放并返回 `{replayed}`
- spool 未配置时返回 503

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

生产多副本、保留、监控与故障处理见 [TIMESERIES_OPERATIONS.md](TIMESERIES_OPERATIONS.md)。

Grafana 仪表板：[deploy/grafana/tdengine-reconciliation-dashboard.json](../deploy/grafana/tdengine-reconciliation-dashboard.json)。

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

### 导出任务

```http
POST /api/integrations/timeseries/reconciliations/export/jobs
GET  /api/integrations/timeseries/reconciliations/export/jobs/{id}
GET  /api/integrations/timeseries/reconciliations/export/jobs/{id}/download
POST /api/integrations/timeseries/reconciliations/export/jobs/{id}/cancel
POST /api/integrations/timeseries/reconciliations/export/jobs/{id}/retry
POST /api/integrations/timeseries/reconciliations/export/jobs/{id}/resume
```

POST body：

```json
{"vehicle_id": "optional", "org_name": "optional", "consistent": false, "max_rows": 5000}
```

- 后台 worker 按 500 行/页写文件
- 状态：`running` / `completed` / `failed`
- 文件目录：`TSP_RECONCILE_EXPORT_DIR`
- 非管理员只能访问本组织任务
- worker 每页检查 `cancelled` 状态，取消后删除临时文件
- `failed` / `cancelled` 任务可重试，重置进度并重新执行
- 重试前会覆盖旧 CSV，避免文件冲突
- `running` 任务超过 `TSP_RECONCILE_EXPORT_STALE_SECS`（默认 60s）可手动 resume
- resume 从持久化 `cursor_ts/cursor_id` 与 `rows_written` 继续追加，不重头导出
- 每个 worker 持有 120s 数据库租约，每页续租，防止多实例重复写
- 每 30s 扫描过期租约并自动恢复 running 任务
- 指标：`xingtu_reconcile_export_jobs_recovered_total`
- 进程重启后任务也会由恢复扫描自动接管
- 指标：`xingtu_reconcile_export_jobs_{started,completed,failed,cancelled}_total`
- 进程重启时 `running` 任务不会自动恢复，需要重新提交
- 已完成/失败任务默认保留 24 小时，每小时清理记录与文件
- `TSP_RECONCILE_EXPORT_RETENTION_HOURS=0` 关闭清理

## 12. 客户端分页导出

```powershell
.\scripts\export-reconciliations.ps1 -Url http://127.0.0.1:8090 -OutFile reconciliations.csv
```

- 自动登录获取 token
- 循环调用 `export/page`
- 校验每页 `X-Row-Count`
- 按 `X-Next-Cursor` 继续
- 默认最多 5000 行
- 若系统禁止脚本执行：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\export-reconciliations.ps1
```

## 13. 测试

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
