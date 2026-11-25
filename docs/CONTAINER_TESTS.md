# 可选存储与中间件容器测试

本页记录**真实容器**验证，与 `src` 内的 mock 微测试分开。mock 用于快速回归；容器测试用于验证协议、认证、网络和版本兼容性。

## 1. 结果矩阵

| 组件 | 容器镜像 | 状态 | 证据 |
| --- | --- | --- | --- |
| PostgreSQL | `postgis/postgis:16-3.4` | 通过 | schema/migrations/seed/车辆/告警 |
| PostGIS | `postgis/postgis:16-3.4` | 通过 | 3.4、geometry、GiST/geography、nearby/bbox |
| PostgreSQL HA | `postgres:16` x2 | 通过 | 流复制、只读从库、手工 promote |
| Patroni HA | `ongres/patroni` + etcd | 通过 | 3 节点自动升主（pg1→pg3）|
| MySQL | `mysql:8.0` | 通过 | schema/migrations/seed/车辆/告警 |
| RustFS | `rustfs/rustfs:latest` | 通过 | 建桶、上传、head-object |
| TDengine | `tdengine/tdengine:3.3.6.0` | 通过 | REST smoke + API trajectory source=tdengine |
| Vault | `hashicorp/vault:latest` | 通过 | dev server + KV v2 同步密钥文件 |
| Iggy | `apache/iggy:latest` | 通过 | HTTP/2 login/stream/topic/publish/poll |
| Redis | `redis:7-alpine` | 通过 | 真实 session SET/GET |
| Walrus | 源码构建 `tsp-walrus:dev` | 通过 | 单节点 + 双节点 PUT/GET |
| tsink | 源码构建 `tsp-tsink:dev` | 通过 | 真实 `/api/v1/import/prometheus` 写入 |
| Kubernetes | k3d v5.8.3 | 通过 | PG+RustFS+TLS+API，1/1 Running，重启数据保留 |

## 2. PostgreSQL + PostGIS

```bash
docker run -d --name tsp-postgis \
  -e POSTGRES_DB=tsp -e POSTGRES_USER=tsp -e POSTGRES_PASSWORD=tsp-dev-password \
  -p 55433:5432 postgis/postgis:16-3.4

docker exec tsp-postgis psql -U tsp -d tsp -c 'CREATE EXTENSION IF NOT EXISTS postgis;'
```

门控测试：

```bash
TSP_TEST_POSTGRES_URL='postgres://tsp:tsp-dev-password@127.0.0.1:55433/tsp?sslmode=disable' \
TSP_TEST_POSTGIS_URL="$TSP_TEST_POSTGRES_URL" \
  cargo test --lib postgres_connection_smoke
TSP_TEST_POSTGIS_URL="$TSP_TEST_POSTGRES_URL" \
  cargo test --lib postgis_extension_is_enabled
```

实测：

```text
postgis_version 3.4 (GEOS=1 PROJ=1 STATS=1)
PostGIS geometry columns and spatial indexes enabled
telemetry geom 264/264 非空
idx_telemetry_geom  GiST (geom)
idx_telemetry_geog  GiST ((geom)::geography)
EXPLAIN 使用 Bitmap Index Scan on idx_telemetry_geog
nearby=3, bbox=3
```

应用健康：

```text
database_engine=postgres
```

### 2.1 PostGIS 10 万/100 万

```bash
psql -U tsp -d tsp -v rows=100000 -f scripts/postgis-bench.sql
psql -U tsp -d tsp -v rows=1000000 -f scripts/postgis-bench.sql
```

实测：

```text
10 万：nearby 938 ms，bbox 48 ms
100 万：nearby 14.6 s，bbox 348 ms
100 万：geom 1000000/1000000，表 578 MB，索引 404 MB
```

两个查询都使用 GiST Bitmap Index Scan。详见 [POSTGIS.md](POSTGIS.md)。

### 2.2 PostgreSQL 主从

```powershell
pwsh -File scripts/postgres-ha-smoke.ps1 -Promote
```

实测：

```text
pg_is_in_recovery = t
pg_stat_replication = streaming|async
lag_bytes = 0
replica SELECT = primary
replica INSERT = read-only 拒绝
promote = 成功
promoted INSERT = ok
```

详见 [POSTGRES_HA.md](POSTGRES_HA.md)。这是手工基线，不是自动 failover。

## 3. MySQL

```bash
docker run -d --name tsp-mysql \
  -e MYSQL_ROOT_PASSWORD=root \
  -e MYSQL_DATABASE=tsp \
  -e MYSQL_USER=tsp \
  -e MYSQL_PASSWORD=tsp-dev-password \
  -p 33060:3306 mysql:8.0

TSP_TEST_MYSQL_URL='mysql://tsp:tsp-dev-password@127.0.0.1:33060/tsp' \
  cargo test --lib mysql_connection_smoke
```

实测：

```text
database_engine=mysql
demo dataset seeded
vehicles total=18
telemetry 写入成功
alerts 写入/查询成功
```

修复：

- `schema_migrations.version` 在 MySQL 必须 `VARCHAR(64)`，`TEXT PRIMARY KEY` 非法。
- MySQL 不支持 `CREATE INDEX IF NOT EXISTS`；schema 启动改为容忍 `1061 duplicate key name`。
- 移除 schema 中与迁移重复的三个索引，避免每次启动告警。

## 4. RustFS

```bash
docker compose -f docker-compose.yml up -d rustfs
```

本地 Docker 单盘需要：

```text
RUSTFS_UNSAFE_BYPASS_DISK_CHECK=true
```

仅限单节点开发；生产多盘不要开启。

实测：

```text
/health 200
s3 mb s3://tsp-media
s3 cp sample.h264
head-object ContentLength=1835
```

## 5. TDengine

```bash
docker compose -f docker-compose.yml --profile tdengine up -d tdengine
./scripts/tdengine-smoke.ps1
```

实测：

```text
server version code=0
database/supertable 创建
insert + count = 1
```

应用：

```text
TSP_TIMESERIES_BACKEND=tdengine
trajectory source=tdengine
TDengine count = 40
```

## 6. Vault

```bash
docker run -d --name tsp-vault -p 8200:8200 \
  -e VAULT_DEV_ROOT_TOKEN_ID=root \
  -e VAULT_DEV_LISTEN_ADDRESS=0.0.0.0:8200 \
  hashicorp/vault:latest server -dev
```

写入：

```json
{"data":{"k1":"QUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUE="}}
```

应用：

```text
TSP_VAULT_ADDR=http://127.0.0.1:8200
TSP_VAULT_TOKEN=root
TSP_VAULT_SECRET_PATH=secret/data/xingtu/export
TSP_EXPORT_ENCRYPTION_KEYS_FILE=<path>
```

实测：

```text
vault export keys synced keys=1
keys_exists=True
```

## 7. Iggy

```bash
docker run -d --name tsp-iggy \
  --security-opt seccomp=unconfined \
  -e IGGY_ROOT_USERNAME=iggy \
  -e IGGY_ROOT_PASSWORD=iggy \
  -e IGGY_HTTP_ADDRESS=0.0.0.0:3000 \
  -p 3000:3000 apache/iggy
```

实测：

```text
TSP_TEST_IGGY_URL=http://127.0.0.1:3000
cargo test --lib iggy_container_roundtrip
```

修复：

1. Iggy 0.10 HTTP listener 是 h2c-first；reqwest 增加 `http2` feature + `http2_prior_knowledge()`。
2. 登录响应为 `access_token: {expiry, token}`，适配器同时兼容字符串与对象。

## 8. Walrus

官方仓库没有可用公开镜像，`nubskr/walrus` / `ghcr.io/nubskr/walrus` 均拉取失败。已从源码构建：

```bash
git clone https://github.com/nubskr/walrus
docker build -f distributed-walrus/Dockerfile -t tsp-walrus:dev .
docker run -d --name tsp-walrus --security-opt seccomp=unconfined \
  -e WALRUS_DISABLE_IO_URING=1 -p 9091:9091 -p 6001:6001 tsp-walrus:dev \
  --node-id 1 --data-dir /data --raft-host 0.0.0.0 \
  --raft-advertise-host 127.0.0.1 --raft-port 6001 \
  --client-host 0.0.0.0 --client-port 9091
```

实测：

```text
TSP_TEST_WALRUS_ADDR=127.0.0.1:9091
walrus_container_put_get ... ok

双节点：
node1 REGISTER OK / PUT OK
node2 REGISTER OK / GET hello-cluster
```

## 9. tsink

`cantrepro/tsink` 没有官方镜像/Dockerfile。已克隆源码，为其 `tsink-server` crate 编写本地 Dockerfile：

```bash
docker build -t tsp-tsink:dev .
# 入口 tsink-server，默认 9201
```

真实 HTTP 测试：

```bash
TSP_TEST_TSINK_URL=http://127.0.0.1:9201 cargo test --lib tsink_container_roundtrip
```

实测：

```text
tsink server listening on 0.0.0.0:9201
tsink_container_roundtrip ... ok
```

注意：

- 容器入口不能传 `server` 子命令，直接传 `--listen/--data-path`。
- `sample()` 的 2023 时间戳会被默认保留窗口拒绝（422 `write_out_of_retention`）；真实测试使用当前时间。
- 端点 `/api/v1/import/prometheus` 要求 label 值带引号。

## 10. Kubernetes 完整链路（k3d）

```bash
k3d cluster create tsp --servers 1 --agents 0 \
  --k3s-arg '--disable=traefik@server:0' --wait
k3d image import xingtu-tsp:k8s -c tsp
# 创建 namespace/secret，apply configmap/postgres/redis/rustfs/api
# 单节点 smoke 配置：cluster=false, postgis=false, api replicas=1
```

实测：

```text
k3d v5.8.3 / k3s v1.31.5
postgres-0 1/1 Running
redis      1/1 Running
rustfs     1/1 Running
tsp-api    1/1 Running
database_engine=postgres
vehicles total=18
JT808 TLS enabled
RustFS media archive enabled
archive prefix media/12345678901/1/
删除 API Pod 后重建：vehicles 仍为 18
```

kind v0.33.0 在同一环境反复失败：

```text
kubeadm wait-control-plane
client rate limiter Wait ... context deadline
```

因此改用 k3d；完整步骤见 [KUBERNETES_SMOKE.md](KUBERNETES_SMOKE.md)。

## 11. 结论

已从 mock 推进到真实容器的组件：MySQL、RustFS、TDengine、Vault、Iggy、Redis、Walrus（双节点）、tsink，以及 Kubernetes 完整链路（k3d）。

仍未完成：多节点 Iggy/tsink、RustFS/TDengine 生产集群、K8s 多副本/托管/Ingress TLS。

不要把 smoke 写成生产验证；上表是当前唯一口径。
