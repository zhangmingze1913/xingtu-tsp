# KingbaseES 容器部署

本目录提供 KingbaseES V8R6 / V9 的容器编排模板。镜像与 License 由人大金仓官方或授权渠道提供，本仓库不附带镜像。

## 1. 前置条件

- Docker Engine / Docker Desktop / 兼容的容器运行时
- 已获取对应架构的 KingbaseES 镜像（V8R6 或 V9）
- 已获取容器版 License 文件（`license.dat`）
- 宿主机端口 `54321` 可用

## 2. 配置

```powershell
Copy-Item deploy/kingbase/.env.example deploy/kingbase/.env
# 编辑 .env
```

至少设置：

```text
KINGBASE_IMAGE=<vendor-registry>/kingbase:v8r6
KINGBASE_LICENSE_FILE=C:\kingbase\license.dat
KINGBASE_PASSWORD=change-me
```

## 3. 启动

```powershell
.\scripts\kingbase-container.ps1 -EnvFile deploy\kingbase\.env
```

等价命令：

```powershell
docker compose --env-file deploy\kingbase\.env -f deploy\kingbase\docker-compose.yml up -d
```

查看：

```powershell
docker logs -f tsp-kingbase
```

## 4. 验证

容器内：

```bash
docker exec -it tsp-kingbase ksql -U tsp -d tsp -c "SELECT version();"
docker exec -i tsp-kingbase ksql -U tsp -d tsp -f /scripts/kingbase-verify.sql
```

宿主机：

```powershell
Test-NetConnection 127.0.0.1 -Port 54321
```

应用：

```powershell
$env:TSP_DATABASE='kingbase://tsp:change-me@127.0.0.1:54321/tsp?sslmode=disable'
$env:TSP_POSTGIS_ENABLED='true'
cargo run --release
```

健康检查：

```powershell
Invoke-RestMethod http://127.0.0.1:8090/api/health
# database_engine=kingbase
```

## 5. KGIS

镜像内是否已带 KGIS 取决于发行包。进入容器执行：

```sql
CREATE EXTENSION IF NOT EXISTS kingbase_spatial;
-- 或
CREATE EXTENSION IF NOT EXISTS postgis;
```

然后重启应用，确认：

```text
schema_migrations 含 0007_kgis_geometry
telemetry.geom 非空
```

## 6. 停止与清理

```powershell
.\scripts\kingbase-container.ps1 -EnvFile deploy\kingbase\.env -Down
```

数据卷 `kingbase-data` 默认保留；删除需显式：

```powershell
docker compose -f deploy/kingbase/docker-compose.yml down -v
```

## 7. 镜像差异

不同厂商/社区镜像的以下路径与变量可能不同：

- License 挂载路径
- 数据目录
- 初始化数据库的环境变量
- 默认用户/密码
- `ksql` 是否在 PATH

本模板使用：

```text
/opt/kingbase/ES/license.dat
/opt/kingbase/ES/data
DB_USER / DB_PASSWORD / DB_NAME
```

如果镜像不兼容，按镜像文档调整 `docker-compose.yml`。

## 8. 当前验证状态

本机没有 Docker/Podman/containerd，WSL 也没有已安装发行版，因此：

- 未实际拉取镜像
- 未启动 KingbaseES 容器
- 未验证 License
- 未执行 KGIS SQL

模板与脚本仅完成静态编写和语法检查。
