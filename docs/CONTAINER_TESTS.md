# 可选存储与中间件容器测试

本页记录**真实容器**验证，与 `src` 内的 mock 微测试分开。mock 用于快速回归；容器测试用于验证协议、认证、网络和版本兼容性。

## 1. 结果矩阵

| 组件 | 容器镜像 | 状态 | 证据 |
| --- | --- | --- | --- |
| MySQL | `mysql:8.0` | 通过 | schema/migrations/seed/车辆/告警 |
| RustFS | `rustfs/rustfs:latest` | 通过 | 建桶、上传、head-object |
| TDengine | `tdengine/tdengine:3.3.6.0` | 通过 | REST smoke + API trajectory source=tdengine |
| Vault | `hashicorp/vault:latest` | 通过 | dev server + KV v2 同步密钥文件 |
| Iggy | `apache/iggy:latest` | 通过 | HTTP/2 login/stream/topic/publish/poll |
| Redis | `redis:7-alpine` | 通过 | 真实 session SET/GET |
| Walrus | 源码构建 `tsp-walrus:dev` | 通过 | 真实 REGISTER/PUT/GET |
| tsink | 源码构建 `tsp-tsink:dev` | 通过 | 真实 `/api/v1/import/prometheus` 写入 |
| Kubernetes | kind v0.33.0 | 通过 | Deployment 1/1 Running，健康检查、登录、车辆 API |

## 2. MySQL

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

## 3. RustFS

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

## 4. TDengine

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

## 5. Vault

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

## 6. Iggy

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

## 7. Walrus

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
```

## 8. tsink

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

## 9. Kubernetes（kind）

```bash
kind create cluster --name tsp-test
kubectl kustomize deploy/k8s >/dev/null   # 清单构建通过

docker build -t xingtu-tsp:k8s .
kind load docker-image xingtu-tsp:k8s --name tsp-test
kubectl apply -f deploy/k8s/smoke-sqlite.yaml
kubectl -n xingtu-tsp-smoke rollout status deploy/tsp-api
kubectl -n xingtu-tsp-smoke port-forward svc/tsp-api 18090:8090
```

实测：

```text
pod 1/1 Running，RESTARTS=0
deployment 1/1 available
/api/health database_engine=sqlite, database=true
login admin/admin123 成功
vehicles 5 / total 18
```

这验证了 namespace/configmap、Deployment、Service、readiness/liveness、emptyDir、runAsUser 10001。

Docker 构建修复：

- Dockerfile 增加 `COPY assets ./assets`；否则 `include_bytes!` 找不到 sample.h264/aac。
- 增加 `CARGO_REGISTRIES_CRATES_IO_PROTOCOL=sparse`，避免容器内 crates.io index 卡住。

完整清单仍使用 PostgreSQL/Redis/RustFS 与 TLS Secret；生产不要用 smoke 清单。

## 10. 结论

已从 mock 推进到真实容器的组件：MySQL、RustFS、TDengine、Vault、Iggy、Redis、Walrus、tsink，以及 Kubernetes smoke。

仍未完成：真实多节点 Iggy/Walrus/tsink 集群、RustFS/TDengine 生产集群、K8s 完整 PostgreSQL/Redis/RustFS/TLS 清单。

不要把 smoke 写成生产验证；上表是当前唯一口径。
