# 集群部署指南

本目录提供 Kubernetes 集群清单与生产部署建议。应用支持两种运行模式：

| 模式 | 数据库 | 实时总线 | 适用 |
| --- | --- | --- | --- |
| 单机演示 | SQLite 文件 | 进程内 broadcast | 本地开发、POC |
| 集群模式 | PostgreSQL | Redis Pub/Sub + 进程内 | 多副本 API / 网关、K8s |

集群模式通过环境变量启用：

```bash
TSP_CLUSTER_MODE=true
TSP_DATABASE=postgres://tsp:***@postgres:5432/tsp?sslmode=disable
TSP_REDIS_URL=redis://redis:6379/0
TSP_NODE_ID=api-0
```

## 1. 前置条件

- Kubernetes 1.28+
- `ingress-nginx`（或云厂商 Ingress）
- `metrics-server`（HPA 需要）
- 可用的 PostgreSQL 14+ 与 Redis 7+
- 已构建并推送镜像

生产环境不要把 `postgres.yaml` / `redis.yaml` 直接当主库主缓存；应使用：

- 云托管 PostgreSQL（RDS/CloudSQL/ PolarDB）、CloudNativePG 或 Patroni 三副本
- 云托管 Redis（ElastiCache/MemoryStore）、Redis Cluster 或 Sentinel
- 对象存储 + 备份策略

`postgres.yaml` 与 `redis.yaml` 仅用于开发/预发。

## 2. 构建镜像

```bash
docker build -t ghcr.io/your-org/xingtu-tsp:0.1.0 .
docker push ghcr.io/your-org/xingtu-tsp:0.1.0
```

或在 `k8s/kustomization.yaml` 中修改 `images`。

## 3. 创建 Secret

复制示例并替换真实值：

```bash
cd deploy/k8s
cp secret.example.yaml secret.yaml
# 编辑 secret.yaml：database-url / jwt-secret / postgres-password
kubectl apply -f namespace.yaml
kubectl -n xingtu-tsp apply -f secret.yaml
```

`database-url` 必须是 SQLx 可识别的 PostgreSQL URL，例如：

```text
postgres://tsp:password@postgres.xingtu-tsp.svc.cluster.local:5432/tsp?sslmode=require
```

生产环境开启 TLS（`sslmode=verify-full`），并把 CA 挂载到容器。

## 4. 部署

```bash
kubectl apply -k deploy/k8s
kubectl -n xingtu-tsp get pods,svc,ingress
```

若使用外部数据库/Redis，删除 `k8s/kustomization.yaml` 中的 `postgres.yaml`、`redis.yaml`，并将 ConfigMap/Secret 中的地址改成外部服务。

> `secret.yaml` 已被 `.gitignore` 忽略，不要提交。

## 5. 验证集群行为

### 5.1 API 多副本与无状态

```bash
kubectl -n xingtu-tsp scale deploy/tsp-api --replicas=3
kubectl -n xingtu-tsp get endpoints tsp-api
```

- 任意 Pod 重启，JWT 仍有效（无本地 session）。
- WebSocket 客户端可落到任意 Pod；Redis Pub/Sub 负责跨 Pod 广播。
- 滚动升级 `maxUnavailable: 0`，PDB 保证至少 2 个副本。

### 5.2 网关多副本

```bash
kubectl -n xingtu-tsp get svc tsp-jt808
# 用返回的 EXTERNAL-IP 作为终端接入地址
```

- 每个 Gateway 只持有部分 TCP 长连接。
- 终端重连落到其他 Pod 后仍可继续上报；数据写入共享 PostgreSQL。
- Redis 记录 `phone -> node` 会话，用于车控下行路由。
- 网关没有本地持久化，滚动升级时终端会自动重连。

### 5.3 选主

模拟器、离线巡检等单例任务通过 Redis 租约选主。任意时刻只有一个 Pod 执行，避免重复产生行程/告警；节点失联后租约 TTL 到期，其他节点接管。

### 5.4 健康检查

```bash
kubectl -n xingtu-tsp port-forward svc/tsp-api 8090:8090
curl -s http://127.0.0.1:8090/api/health
```

数据库不可用时 `/api/health` 返回 `503`，readiness 失败，Ingress 自动摘除该 Pod；恢复后重新加入。

## 6. 扩缩容

- API：HPA 按 CPU/内存 3–20 副本。
- Gateway：建议按**连接数**自定义指标扩容，CPU 只能作为兜底；连接数 2–5 万/Pod 估算。
- PostgreSQL/Redis/Kafka：使用独立运维体系，不跟业务 Pod 一起 HPA。

## 6.1 MySQL 模式

`TSP_DATABASE` 也接受 `mysql://user:password@host:3306/tsp`。MySQL 可用于单机或 `TSP_CLUSTER_MODE=true` 的共享业务库（配合 Redis 选主/会话）；首次种子使用 `GET_LOCK` 串行化。MySQL 侧使用 InnoDB、utf8mb4，`?N` 占位符由数据层按出现顺序重写为位置参数。

## 6.2 Apache Iggy 事件流

默认使用 Redis Pub/Sub。要启用持久事件日志：

```bash
kubectl -n xingtu-tsp apply -f deploy/k8s/iggy.yaml
# 然后在 api 与 gateway 两个 Deployment 上设置：
# TSP_IGGY_URL=http://iggy:3000
# TSP_IGGY_USERNAME / TSP_IGGY_PASSWORD
```

Iggy HTTP API 需要 3000 端口；Redis 仍负责选主与网关会话。Iggy 的 `xingtu-tsp/events` stream/topic 会在启动时自动创建，每个节点使用独立 consumer 读取同一分区。生产建议使用 Iggy 集群/持久卷，并配置认证与 TLS。

本地验证：

```bash
docker compose -f deploy/docker-compose.cluster.yml -f deploy/docker-compose.iggy.yml up --build
```

## 6.3 OpenRaft 控制面
```bash
export TSP_RAFT_ENABLED=true
export TSP_RAFT_NODE_ID=1
export TSP_RAFT_ADDR=0.0.0.0:7001
export TSP_RAFT_PEERS='tsp-api-0.tsp-api-headless:7001,tsp-api-1.tsp-api-headless:7001,tsp-api-2.tsp-api-headless:7001'
export TSP_RAFT_BOOTSTRAP=true
```

- 每个节点使用固定 `TSP_RAFT_NODE_ID` 和可达的 `TSP_RAFT_ADDR`
- K8s 生产使用 `api-statefulset.example.yaml`（StatefulSet + headless Service）；peer 可写 `host:port`，节点 ID 由稳定主机名前缀派生
- Raft RPC 走 JSON over HTTP，端口需在 NetworkPolicy 中放行
- `TSP_RAFT_DATA_DIR` 持久化 log/vote/committed 与状态机 JSON（StatefulSet 示例挂 PVC）；快照文件尚未单独持久化
- 关闭 `TSP_RAFT_ENABLED` 即回退到 Redis 租约选主

## 6.4 tsink 时序库与 Walrus 归档

tsink（Prometheus 兼容 HTTP，默认 9201）：

```bash
# 任选一种方式运行 tsink server
tsink-server --listen 0.0.0.0:9201 --data-path /data/tsink
# 应用侧
export TSP_TSINK_URL=http://tsink:9201
```

应用向 `/api/v1/import/prometheus` 写入 `tsp_lat/tsp_lon/tsp_speed/tsp_heading/tsp_soc/tsp_odometer`，轨迹查询走 `/api/v1/query_range`；不可用时自动回退 PostgreSQL/MySQL/SQLite。

TDengine 3.x（taosAdapter REST，默认 6041）：

```bash
export TSP_TDENGINE_URL=http://tdengine:6041
export TSP_TDENGINE_USER=root
export TSP_TDENGINE_PASSWORD=taosdata
export TSP_TDENGINE_DATABASE=tsp
```

应用启动时自动执行 `CREATE DATABASE` 与 `CREATE STABLE telemetry`，每台车一个子表；轨迹查询使用 `WHERE vehicle_id = ... AND ts BETWEEN ...`。查询优先级为 tsink → TDengine → 关系库。

Walrus（长度前缀 TCP，默认 client port 8080）：

```bash
export TSP_WALRUS_ADDR=walrus:8080
export TSP_WALRUS_TOPIC=tsp-events
```

`EventBus` 会将每个领域事件 JSON 以 `PUT <topic> <payload>` 写入 Walrus 归档。Walrus topic 使用共享 cursor，因此本适配器**只做持久归档/下游流，不做多节点 WebSocket 广播**；实时广播仍由 Redis/Iggy 承担。上游 Walrus 服务端的存储后端面向 Unix，Windows 本地无法启动，实际部署请使用 Linux/Docker。

自定义指标示例（Prometheus Adapter）：

```yaml
metrics:
  - type: Pods
    pods:
      metric:
        name: xingtu_jt808_connections
      target:
        type: AverageValue
        averageValue: "30000"
```

## 6.5 PostGIS（可选）

- PostgreSQL 需预装 PostGIS：`CREATE EXTENSION IF NOT EXISTS postgis;`
- `TSP_POSTGIS_ENABLED=true`（默认）时，应用自动增加 `telemetry.geom` / `vehicle_state.geom`、回填经纬度并建 GiST 索引
- 扩展不可用会告警并降级为 lat/lon 查询，不影响启动
- 新库或已有库都通过启动迁移处理；生产建议由 DBA 预创建扩展并 `ANALYZE`

## 7. 可观测性

- `/api/metrics`（Prometheus 文本格式）：HTTP 请求、车辆/告警计数、WS 连接数、网关连接数。
- 日志：JSON 结构化，输出到 stdout，由 Loki/ELK 采集。
- 链路：接入 trace_id → Kafka → 业务服务 → DB。
- 告警：API 5xx、网关连接数骤降、Kafka lag、PG 复制延迟、Redis 故障。

`api.yaml` 已包含 `prometheus.io/scrape` 注解；如使用 Prometheus Operator，请添加 ServiceMonitor。

## 8. 数据与备份

- PostgreSQL：每日全量 + WAL 归档（PITR），至少保留 14 天；每季度恢复演练。
- Redis：只存会话/在线态/锁，可丢失；生产开启 AOF + 主从。
- 轨迹：Timescale/ClickHouse 分区 + 对象存储归档；不要用 `DELETE` 清理大表，按分区 drop。
- 配置与密钥：GitOps + Secret Manager；不要放在镜像或 ConfigMap 明文。
- **Schema**：当前应用启动时执行 `CREATE TABLE IF NOT EXISTS`，没有版本化 ALTER 迁移；生产升级请使用独立 migration Job（Flyway/Liquibase/sqlx-cli）并遵循 expand/contract。
- **首次种子**：多副本同时首次启动可能竞争 demo seed。生产环境建议由一次性 Job 完成初始化，或设置 `TSP_DEMO_MODE=false` 并预先导入租户数据。

## 9. 安全清单

- Ingress 强制 HTTPS，HSTS；WebSocket 使用 WSS。
- PostgreSQL/Redis 只开内网访问，NetworkPolicy 限制来源。
- 网关 L4 LB 仅开放 18080，源 IP 白名单（运营商专网/VPN）。
- JWT Secret ≥ 32 字节，定期轮换；支持双密钥灰度。
- 高危车控双人复核、操作审计、速率限制。
- 镜像签名/扫描，非 root、只读根文件系统。
- 终端侧 mTLS/证书或专网接入，防止伪造终端。

## 9.1 生产安全基线（新增）

- `TSP_ENV=production`、`TSP_DEMO_MODE=false`、`TSP_COMMAND_MODE=disabled|jt808`；禁止模拟指令。
- 设置 ≥32 字节 `TSP_JWT_SECRET`；空库首启设置 `TSP_ADMIN_PASSWORD`。
- `TSP_CORS_ORIGINS` 只列可信前端域名；不要留空泛 Any。
- WebSocket 使用 `Sec-WebSocket-Protocol: bearer.<JWT>`，不要再用 URL query token。
- 审计日志保留 180 天，经 `/api/audit` 查询、`/api/audit/export` 导出；生产建议再落 WORM/对象存储。
- operator/viewer 按 `users.org_name` 做组织域隔离；管理员跨组织。
- JT/T 808 生产必须置于 VPN/专线或 TLS 终结之后，终端侧补 mTLS/证书。

## 10. 回滚

```bash
kubectl -n xingtu-tsp rollout undo deploy/tsp-api
kubectl -n xingtu-tsp rollout status deploy/tsp-api
```

数据库 schema 变更遵循 expand/contract：先加列/索引，再切流量，最后删除旧列；禁止在发布窗口执行大表锁操作。

## 11. 单机快速验证集群代码

没有 K8s 时，可用 `deploy/docker-compose.cluster.yml` 一次启动 2 个 API、2 个网关、PostgreSQL、Redis 与 Nginx：

```bash
docker compose -f deploy/docker-compose.cluster.yml up --build
```

控制台：`http://127.0.0.1:8090`，JT/T 808：`127.0.0.1:18080`。该拓扑仅用于本地验证，不用于生产。

没有 Docker 时，可用环境变量在本机验证多进程：

```bash
# terminal 1: PostgreSQL + Redis via your local tooling
export TSP_DATABASE='postgres://tsp:tsp@127.0.0.1:5432/tsp?sslmode=disable'
export TSP_REDIS_URL='redis://127.0.0.1:6379/0'
export TSP_CLUSTER_MODE=true
TSP_BIND=127.0.0.1:8090 TSP_JT808_ADDR=127.0.0.1:18080 TSP_NODE_ID=node-a cargo run

# terminal 2
TSP_BIND=127.0.0.1:8091 TSP_JT808_ADDR=127.0.0.1:18081 TSP_NODE_ID=node-b cargo run
```

两个实例共享同一 PostgreSQL/Redis；用 Nginx/HAProxy 将 8090/8091 与 18080/18081 做负载均衡即可验证跨实例实时推送与选主。
