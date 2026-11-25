# TSP 系统初始化

本页对应可直接导出的初始化脚本：

- Windows PowerShell：[init-system.ps1](../scripts/init-system.ps1)
- Linux/macOS Bash：[init-system.sh](../scripts/init-system.sh)

脚本负责生成 `.env`、JWT/JT808 密钥、PII/导出密钥和 SQLite 数据目录；不负责安装 Rust、启动数据库或启动进程。

## 1. Windows

```powershell
# 本地 SQLite 演示
.\scripts\init-system.ps1

# 覆盖已有 .env（先自动备份）
.\scripts\init-system.ps1 -Force

# PostgreSQL
.\scripts\init-system.ps1 -Database postgres `
  -DatabaseUrl 'postgres://tsp:password@127.0.0.1:5432/tsp?sslmode=disable'

# 生产：生成随机管理员密码、关闭 demo、启用 TLS
.\scripts\init-system.ps1 -Production -AdminPassword '从密钥管理系统获取'
```

可选：

```powershell
-Build        # cargo build --release
-Run          # 加载 .env 后 cargo run --release
-EnvFile      # 自定义 .env 路径
-DataDir      # SQLite 数据目录
```

## 2. Linux / macOS

```bash
chmod +x scripts/init-system.sh

./scripts/init-system.sh

./scripts/init-system.sh --db postgres \
  --url 'postgres://tsp:password@127.0.0.1:5432/tsp?sslmode=disable'

./scripts/init-system.sh --production \
  --admin-password '从密钥管理系统获取' --build
```

参数：

```text
--db sqlite|postgres|mysql|kingbase
--url <database-url>
--env-file <path>
--data-dir <path>
--admin-password <value>
--force
--production
--build
--run
```

## 3. 生成内容

默认生成：

```text
.env
data/
secrets/
  jwt-private.pem
  jwt-public.pem
  jt808-key.pem
  jt808-cert.pem
```

`.env` 包含：

```text
TSP_DATABASE
TSP_DEMO_MODE
TSP_POSTGIS_ENABLED
TSP_JWT_SECRET / KEY_ID
TSP_JWT_PRIVATE_KEY_FILE / PUBLIC
TSP_JT808_TLS_CERT / KEY
TSP_PII_KEY
TSP_EXPORT_TOKEN_SECRET
TSP_EXPORT_ENCRYPTION_KEY
```

- 开发模式：`admin/admin123`，demo 数据开启。
- 生产模式：随机管理员密码、demo 关闭、JT808 不允许明文。
- 生产模式必须使用 PostgreSQL/KingbaseES/MySQL，不允许 SQLite。
- Windows 未安装 OpenSSL 时会退回进程内开发 JWT 密钥并警告；生产模式会直接失败。

## 4. 安全

```text
.env、secrets/ 已加入 .gitignore
Linux 脚本设置 .env 0600、secrets/ 0700
生产密码只打印一次，请立即保存到 Vault/KMS/Secret Manager
不要提交 jwt-private.pem、jt808-key.pem
```

## 5. 初始化后

```bash
cargo run --release
curl http://127.0.0.1:8090/api/health
```

生产：

```text
设置 TSP_ENV=production
配置 Redis/TLS/JWT/JT808
执行数据库迁移/种子
按 OPERATIONS_GUIDE 做备份与监控
```

## 6. 数据库前置

脚本不会自动启动数据库。PostgreSQL 示例：

```bash
docker run -d --name tsp-postgis \
  -e POSTGRES_DB=tsp -e POSTGRES_USER=tsp -e POSTGRES_PASSWORD=tsp-dev-password \
  -p 55433:5432 postgis/postgis:16-3.4

docker exec tsp-postgis psql -U tsp -d tsp -c 'CREATE EXTENSION IF NOT EXISTS postgis;'
```

MySQL：

```bash
docker run -d --name tsp-mysql \
  -e MYSQL_ROOT_PASSWORD=root -e MYSQL_DATABASE=tsp \
  -e MYSQL_USER=tsp -e MYSQL_PASSWORD=tsp-dev-password \
  -p 33060:3306 mysql:8.0
```

KingbaseES 见 [KINGBASE_KGIS.md](KINGBASE_KGIS.md)，必须 `DB_MODE=pg`。

## 7. 验证记录

Windows：

```text
init-system.ps1 -EnvFile D:\tsp-init-test.env -DataDir D:\tsp-init-data -Force
结果：.env、data、secrets 生成，SQLite URL 正确
```

Linux（postgres:16 容器内）：

```text
init-system.sh --env-file /tmp/tsp-test.env --data-dir /tmp/tsp-data --force
结果：.env、data 生成
```

生产模式（Bash）：

```text
--production --db postgres --admin-password ... 
结果：TSP_ENV=production、demo 关闭、JWT/JT808 密钥生成
```

## 8. 注意

- 脚本不替代数据库/Redis/RustFS 部署。
- 生产模式只是配置前置，不代表生产就绪。
- 初始化后仍需按 [PRODUCTION_READINESS.md](PRODUCTION_READINESS.md) 完成容量、HA、安全、终端与合规验证。
