# 星途 TSP 运维手册

## 1. 部署形态

| 形态 | 数据库 | 适用 |
| --- | --- | --- |
| 单机演示 | SQLite | 本地演示、开发、验收 |
| 单机生产 | PostgreSQL/KingbaseES | 中小车队 |
| 集群 | PostgreSQL/KingbaseES + Redis | 多副本 API/网关 |
| 大型轨迹 | 上述 + TDengine | 轨迹查询扩容 |

## 2. 端口

| 端口 | 用途 |
| --- | --- |
| 8090/8091/8092 | API / 控制台 |
| 18080/18081/18082 | JT/T 808 TCP 网关 |
| 19000/udp | JT/T 1078 媒体 |
| 5432 | PostgreSQL（默认） |
| 54321 | KingbaseES |
| 6379/56379 | Redis |
| 9000/9001 | RustFS API/Console |
| 6041 | TDengine taosAdapter |

## 3. 启动

### 3.1 单机 Demo

```bash
docker compose up -d tsp
```

或本地：

```bash
TSP_DATABASE=data/tsp.db TSP_DEMO_MODE=true cargo run --release
```

默认 `admin/admin123`。

### 3.2 集群 smoke

```bash
docker compose -f deploy/docker-compose.cluster.yml up -d
```

包含 PostgreSQL、Redis、两 API、两网关、Nginx。

### 3.3 KingbaseES

```bash
docker compose -f deploy/kingbase/docker-compose.embedded-license.yml up -d kingbase-v8
```

然后：

```bash
TSP_DATABASE='kingbase://tsp:***@127.0.0.1:54321/tsp?sslmode=disable'
TSP_POSTGIS_ENABLED=true
```

必须 `DB_MODE=pg`；不要用默认 Oracle 模式。

### 3.4 TDengine

```bash
docker compose --profile tdengine up -d tdengine
```

详见 [TIMESERIES_OPERATIONS.md](TIMESERIES_OPERATIONS.md)。

### 3.5 RustFS

```bash
docker compose up -d rustfs
```

首次启动会创建 bucket；归档开关 `TSP_RUSTFS_ARCHIVE=true`。

## 4. 健康与可观测

```bash
curl http://127.0.0.1:8090/api/health
curl http://127.0.0.1:8090/api/ready
curl http://127.0.0.1:8090/api/metrics
```

`/api/health` 关键字段：

```json
{
  "database": true,
  "database_engine": "postgres|kingbase|mysql|sqlite",
  "timeseries_backend": "postgres|tdengine|dual",
  "tdengine": false,
  "tdengine_ready": null
}
```

`/api/ready` 在 `tdengine` 模式下要求 TDengine 探测成功；`dual` 不阻塞。

Prometheus 抓取 `/api/metrics`。不要对 `/api/metrics` 做公网暴露。

## 5. 日志

```bash
docker logs -f xingtu-tsp
```

建议级别：

```text
生产   RUST_LOG=xingtu_tsp=info,openraft=warn
排障   RUST_LOG=xingtu_tsp=debug,sqlx=warn
```

不要长期开 `sqlx=debug`，会记录 SQL 与参数。

## 6. 数据库运维

### PostgreSQL 备份

```powershell
.\scripts\backup-postgres.ps1 -PgBin <pg_bin> -OutDir backups
```

恢复：

```powershell
.\scripts\restore-postgres.ps1 -File backups\... -PgBin <pg_bin>
```

每次备份生成 `.manifest.json`，包含 SHA-256。

### PostgreSQL 主从

手工流复制/promote smoke：

```powershell
pwsh -File scripts/postgres-ha-smoke.ps1 -Promote
```

详见 [POSTGRES_HA.md](POSTGRES_HA.md)。这是手工基线，不是自动 failover。

### KingbaseES 备份

使用 `sys_dump` / `ksql`，按厂商工具执行。应用侧只保证 schema 兼容；备份策略由 KES 平台负责。

### 迁移

应用启动自动执行 `schema_migrations`。升级前：

1. 备份
2. 在预发执行 `cargo test`
3. 启动新版本，观察迁移日志
4. 不要手工删 `schema_migrations`

## 7. 容量与保留

- `telemetry`：按车辆/时间索引，定期 `VACUUM (ANALYZE)`
- `pii_vault`：只存密文
- `reconciliation_export_jobs` + `export_alert_state`：默认 24h/90d 清理
- RustFS 归档：`TSP_RUSTFS_RETENTION_DAYS`，0 表示不自动删
- TDengine：`KEEP`/`DURATION` 独立设置

容量建议见 [CAPACITY.md](CAPACITY.md) 和 [RECOMMENDATIONS.md](RECOMMENDATIONS.md)。

## 8. 常见故障

### 应用启动失败：数据库

```text
检查 TSP_DATABASE
检查端口/账号/sslmode
Kingbase：DB_MODE=pg，ENABLE_CI=no
```

### Kingbase：`null value in column org_name`

镜像运行在 Oracle 模式，空字符串变 NULL。重建为：

```bash
DB_MODE=pg ENABLE_CI=no
```

### KGIS 未启用

```text
WARN spatial extension is not installed
```

应用降级 lat/lon。要启用 GIS，需要安装 KGIS 插件包或使用 `kes-spatial` 镜像。

### TDengine 写入失败

检查 `/api/metrics`：

```text
xingtu_tdengine_failures_total
xingtu_tdengine_spool_pending
xingtu_tdengine_queue_depth
```

spool 接口：

```http
GET /api/integrations/timeseries/spool
POST /api/integrations/timeseries/spool/replay
```

### 归档上传失败

检查 RustFS 健康、bucket、AK/SK；本地 spool 在 `TSP_ARCHIVE_SPOOL_DIR`。

### 登录慢

Argon2 本身耗时；确认使用 release 构建，且没有在 async worker 内同步校验（当前已 `spawn_blocking`）。

## 9. 安全运维

- 修改默认 `admin` 密码
- 生产设置独立 `TSP_JWT_PRIVATE_KEY_FILE` / `TSP_JWT_PUBLIC_KEY_FILE`
- 开启 JT808 TLS/mTLS
- Redis 使用 AUTH/TLS
- 导出加密密钥通过 Secret/Vault 挂载，权限 0400/0600
- 不要把 `.env`、`license.dat`、私钥提交到 Git
- 限制 `/api/metrics`、`/api/audit`、导出接口的访问

## 10. 升级与回滚

升级：

1. 备份数据库
2. 预发启动新版本，确认迁移
3. 滚动更新 API/网关
4. 检查 `/api/ready` 与错误率
5. 观察 15 分钟

回滚：

1. 切回旧镜像
2. 数据库迁移不自动回滚；若迁移不兼容，恢复备份
3. 先回滚 API，再回滚网关

## 11. 交付检查

上线前执行：

```powershell
.\scripts\release-preflight.ps1
```

并对照 [DELIVERY_CHECKLIST.md](DELIVERY_CHECKLIST.md)。

## 12. 当前未验证项

- 真实 TDengine 集群
- 真实 Alertmanager/Vault
- KGIS 空间扩展
- RustFS 实际归档/播放
- 百万级当前版本复测

这些必须在目标环境执行，不能用本地 mock 代替。


## 13. SLO 与告警建议

| 指标 | 建议阈值 | 说明 |
| --- | --- | --- |
| API 5xx 比例 | < 1% / 5min | 数据库/依赖 |
| `/api/ready` | 连续 3 次失败 | 摘流量 |
| 登录 p95 | < 1s | Argon2/DB |
| 车辆列表 p95 | < 300ms | 10K 级 |
| dashboard 缓存命中 | > 90% | 2s TTL |
| DB 连接池使用率 | < 80% | 避免打满 |
| TDengine queue | < 50% | 写入堆积 |
| TDengine spool | 不持续增长 | 重放失败 |
| 复制延迟 | < 5s | 只读副本 |
| 磁盘 | < 80% | WAL/归档/导出 |

告警分级：

```text
P1  API 全不可用 / 数据库不可写 / 远控异常
P2  TDengine 持续失败 / 复制延迟 / 归档堆积
P3  单实例重启 / 导出失败 / 磁盘 70%
```

## 14. 故障演练清单

```text
[ ] 停掉一个 API，LB 摘除，剩余实例正常
[ ] 停掉 Redis，单机降级/集群行为符合预期
[ ] 停掉 TDengine（dual），spool 增长并可重放
[ ] 停掉 RustFS，spool 保留并可恢复
[ ] 主库只读/不可用，应用明确报错
[ ] 从库延迟，只读查询不冒充强一致
[ ] 导出任务中断，租约恢复
[ ] 磁盘满，写入失败但不破坏已有数据
```

## 15. 部署检查

```text
[ ] TSP_ENV=production
[ ] 修改 admin 密码
[ ] JWT 私钥/公钥持久化
[ ] JT808 TLS/mTLS
[ ] Redis TLS/AUTH
[ ] PostgreSQL/Kingbase 备份
[ ] 导出密钥/下载密钥
[ ] Prometheus 抓取
[ ] 日志集中化
[ ] 资源 limits/requests
[ ] PDB/HPA
[ ] 回滚镜像
```

## 16. 相关文档

- [QUICKSTART.md](QUICKSTART.md)
- [TECHNICAL_SOLUTION.md](TECHNICAL_SOLUTION.md)
- [ARCHITECTURE_DIAGRAMS.md](ARCHITECTURE_DIAGRAMS.md)
- [POSTGRES_HA.md](POSTGRES_HA.md)
- [CONTAINER_TESTS.md](CONTAINER_TESTS.md)
