# TDengine 运维：多副本、保留与故障处理

本文覆盖 TDengine 作为 `tdengine` / `dual` 后端时的生产运维边界。应用侧选型见 [TIMESERIES_BACKENDS.md](TIMESERIES_BACKENDS.md)。

## 1. 部署拓扑

### 评估/预发

- 单节点 `tdengine/tdengine:3.3.6.0`
- 单盘或本地盘
- `deploy/k8s/tdengine.yaml`
- `docker compose --profile tdengine`

### 生产

建议 3 节点 TDengine 集群：

| 组件 | 数量 | 说明 |
| --- | --- | --- |
| taosd | 3 | 数据节点，端口 6030 |
| taosAdapter | 每节点 | REST 6041，供应用访问 |
| mnode | 3 | 管理节点，奇数 |
| taosKeeper | 1–2 | 监控指标采集 |
| 负载均衡/VIP | 1 | REST 6041 入口 |

应用当前只配置**一个** `TSP_TDENGINE_URL`。多节点时请在其前面放 L4/L7 负载均衡或 VIP，不要直接写死单节点地址。

TDengine 副本数由数据库 `REPLICA` 决定：

```sql
CREATE DATABASE tsp KEEP 365 DURATION 10 REPLICA 3;
```

`REPLICA 3` 要求至少 3 个可用 dnode；单节点环境不要设置。

## 2. 容量与保留

### 保留

建库时指定：

```sql
CREATE DATABASE tsp KEEP 365 DURATION 10;
```

调整：

```sql
ALTER DATABASE tsp KEEP 180;
```

- `KEEP`：数据保留天数
- `DURATION`：文件分组时间跨度
- 过期数据由 TDengine 自动清理
- PostgreSQL 侧仍有自己的 `prune_telemetry_before` 策略，两者应独立评估

### 容量估算

按样本粗算：

```text
每行约 60–100 字节（含压缩前开销）
每日行数 = 在线车辆 × 每车每日样本数
原始日增 = 每日行数 × 每行字节
压缩后通常显著低于原始值，取决于列基数/重复率
```

示例：

| 在线车辆 | 每车 1 样本/10s | 日行数 | 保留 180 天行数 |
| --- | --- | --- | --- |
| 10,000 | 8,640 | 86.4M | 15.5B |
| 100,000 | 8,640 | 864M | 155B |
| 100,000 | 1 样本/30s | 288M | 51.8B |

实际磁盘必须用 `scripts/tdengine-bench.ps1` 在目标机型上测量，不要用上表直接采购。

### 分区/超级表

当前每个车辆一个子表 `v_<sanitized_id>`，超级表按 `vehicle_id` 标签过滤。车辆数很大时关注：

- 子表数量与元数据开销
- `KEEP` 与磁盘水位
- 查询是否总带 `vehicle_id` + 时间范围

## 3. 应用配置

```bash
TSP_TIMESERIES_BACKEND=tdengine
TSP_TDENGINE_URL=http://tdengine-vip:6041
TSP_TDENGINE_DATABASE=tsp
TSP_TDENGINE_QUEUE_CAPACITY=8192
TSP_TDENGINE_WRITE_RETRIES=3
TSP_TDENGINE_SPOOL_DIR=/data/tdengine-spool
TSP_TIMESERIES_RECONCILE_ENABLED=true
```

REST 地址必须是 taosAdapter，不是 6030 原生端口。

## 4. 监控与告警

应用指标：

```text
xingtu_tdengine_writes_total
xingtu_tdengine_failures_total
xingtu_tdengine_retries_total
xingtu_tdengine_dropped_total
xingtu_tdengine_queue_depth
xingtu_tdengine_spool_pending
xingtu_tdengine_spool_failed_total
xingtu_tdengine_reconcile_checks_total
xingtu_tdengine_reconcile_mismatches_total
```

建议告警：

| 指标 | 条件 | 动作 |
| --- | --- | --- |
| `queue_depth` | 持续 > 50% 容量 | 扩容/降采样 |
| `failures_total` | 5 分钟增长 > 0 | 检查 taosAdapter/磁盘 |
| `spool_pending` | 持续增长 | 检查 TDengine 写入 |
| `reconcile_mismatches_total` | 增长 | 查双写延迟/丢失 |
| `tdengine_ready` | false | 检查 REST/集群 |

Grafana 仪表板：

```text
deploy/grafana/tdengine-reconciliation-dashboard.json
```

导入 Grafana 后选择 Prometheus 数据源，包含：

- 对账 checks/mismatches
- 队列深度
- spool pending / replay
- 告警 sent/failed/skipped
- 写入/失败/重试
- 丢弃与历史清理

TDengine 自身：

- `taosKeeper` + Prometheus
- taosd 日志
- 磁盘水位、dnode/mnode 状态：

```sql
SHOW DNODES;
SHOW MNODES;
SHOW VGROUPS;
```

## 5. 对账

手动：

```http
GET /api/integrations/timeseries/reconcile?vehicle_id=...&from=...&to=...
```

定时：

```bash
TSP_TIMESERIES_RECONCILE_ENABLED=true
TSP_TIMESERIES_RECONCILE_SECS=300
TSP_TIMESERIES_RECONCILE_WINDOW_MINUTES=15
TSP_TIMESERIES_RECONCILE_MAX_VEHICLES=100
```

- 仅 leader 执行
- 计数级，不替代逐点校验
- 不一致只写日志与指标，未接 Alertmanager/PagerDuty

## 6. 故障处理

### TDengine 不可用

`tdengine` 模式：

- `/api/ready` 失败
- 查询回退 PostgreSQL
- 写入进入重试/队列/spool

`dual` 模式：

- 查询回退 PostgreSQL
- 应用继续运行

### spool 积压

1. 检查 `xingtu_tdengine_queue_depth` 与 `spool_pending`
2. 检查 taosAdapter 健康与磁盘
3. 修复后后台每 60s 重放
4. 不要手工删除 spool 文件，除非确认数据可丢

### 磁盘满

- TDengine 可能拒绝写入
- 先扩盘/调小 `KEEP`
- 检查 spool 所在卷
- 不要直接删除 taosd 数据目录

## 7. 备份与恢复

- 使用 `taosdump` 做逻辑备份
- 备份数据库 schema 与关键保留策略
- 恢复后用 `COUNT(*)`/时间范围与应用对账
- PostgreSQL 仍是权威源，TDengine 可按需重建

## 8. 升级/回滚

1. 先升级一个 dnode，观察 `SHOW DNODES`
2. 再升级其余节点
3. 应用侧保持 `dual`，验证查询与对账
4. 回滚时先切 `TSP_TIMESERIES_BACKEND=postgres`，再降级 TDengine

## 9. 明确限制

- 应用只支持一个 REST URL，无客户端多节点故障转移
- 没有 TDengine 写入 outbox/逐点校验
- 没有自动容量规划
- 定时对账每次最多 100 台
- 真实 TDengine 集群/百万级压测未在本环境执行
