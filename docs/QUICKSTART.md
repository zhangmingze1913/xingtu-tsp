# 快速开始

## 1. 本地运行（SQLite）

要求：

```text
Rust 1.94+
MSVC Build Tools（Windows）或 build-essential（Linux）
```

```bash
cp .env.example .env
cargo run --release
```

打开：

```text
http://127.0.0.1:8090
```

演示账号：

```text
admin / admin123
operator / operator123
viewer / viewer123
```

健康检查：

```bash
curl http://127.0.0.1:8090/api/health
```

期望：

```json
{"database":true,"database_engine":"sqlite"}
```

## 2. Docker Compose

```bash
docker compose up -d tsp
```

包含：

- API/控制台 `:8090`
- JT/T 808 `:18080`
- RustFS `:9000`

停止：

```bash
docker compose down
```

## 3. PostgreSQL + PostGIS

```bash
docker run -d --name tsp-postgis \
  -e POSTGRES_DB=tsp \
  -e POSTGRES_USER=tsp \
  -e POSTGRES_PASSWORD=tsp-dev-password \
  -p 55433:5432 postgis/postgis:16-3.4

docker exec tsp-postgis psql -U tsp -d tsp -c 'CREATE EXTENSION IF NOT EXISTS postgis;'
```

创建 `.env`：

```bash
TSP_DATABASE=postgres://tsp:tsp-dev-password@127.0.0.1:55433/tsp?sslmode=disable
TSP_POSTGIS_ENABLED=true
```

## 4. MySQL

```bash
docker run -d --name tsp-mysql \
  -e MYSQL_ROOT_PASSWORD=root \
  -e MYSQL_DATABASE=tsp \
  -e MYSQL_USER=tsp \
  -e MYSQL_PASSWORD=tsp-dev-password \
  -p 33060:3306 mysql:8.0
```

```bash
TSP_DATABASE=mysql://tsp:tsp-dev-password@127.0.0.1:33060/tsp
```

## 5. KingbaseES

```bash
docker compose -f deploy/kingbase/docker-compose.embedded-license.yml up -d kingbase-v8
```

```bash
TSP_DATABASE='kingbase://tsp:tsp-dev-password@127.0.0.1:54321/tsp?sslmode=disable'
TSP_POSTGIS_ENABLED=true
```

注意 `DB_MODE=pg`，不要使用 Oracle 模式。

## 6. 验证

```bash
# 登录
curl -s -X POST http://127.0.0.1:8090/api/auth/login \
  -H 'Content-Type: application/json' \
  -d '{"username":"admin","password":"admin123"}'

# 车辆列表（替换 token）
curl -s http://127.0.0.1:8090/api/vehicles?page_size=5 \
  -H 'Authorization: Bearer <access_token>'
```

## 7. 常见问题

### 登录失败

- 演示模式是否 `TSP_DEMO_MODE=true`
- 数据库是否已 seed
- 生产是否设置了 `TSP_ADMIN_PASSWORD`

### 没有轨迹

- 是否上报过 `/api/telemetry`
- 时间范围是否正确
- TDengine 模式是否启动失败

### 没有视频

- 是否配置 RustFS
- `TSP_RUSTFS_ARCHIVE=true` 是否开启
- 1078 UDP 端口是否可达

### 空间查询慢

- 是否启用 PostGIS
- geom 是否有 GiST 索引
- 100 万级 nearby 需要时间窗/分区

## 8. 下一步

- 使用手册：[USER_GUIDE.md](USER_GUIDE.md)
- 技术方案：[TECHNICAL_SOLUTION.md](TECHNICAL_SOLUTION.md)
- 运维：[OPERATIONS_GUIDE.md](OPERATIONS_GUIDE.md)
- 架构图：[ARCHITECTURE_DIAGRAMS.md](ARCHITECTURE_DIAGRAMS.md)

> 当前是参考实现/POC，不是生产合规成品。生产前必须完成安全、容量、终端和第三方检测。
