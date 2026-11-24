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
  "tdengine": true
}
```

`tdengine=false` 表示未启用或初始化失败。

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
- 重试耗尽：记录失败指标
- 指标：

```text
xingtu_tdengine_writes_total
xingtu_tdengine_failures_total
xingtu_tdengine_retries_total
xingtu_tdengine_dropped_total
xingtu_tdengine_queue_depth
```

## 8. 一致性与限制

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

## 9. 测试

代码侧测试：

```text
tdengine::tests::parses_timestamps_and_identifiers
tdengine::tests::rest_query_maps_rows_to_points
tdengine::tests::write_retries_transient_failures
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
