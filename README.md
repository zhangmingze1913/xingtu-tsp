# 星途车联 · TSP 智能运营平台（Rust）

面向车联网 TSP（Telematics Service Provider）场景的可运行运营平台：统一接入车辆、T-Box、位置与告警数据，提供实时监控、电子围栏、行程分析、远程车控和运营看板。

后端使用 **Rust + Axum + Tokio + SQLx**，数据库支持 **SQLite（单机）/ PostgreSQL / MySQL（集群或单机）**，实时总线支持**进程内 broadcast / Redis Pub/Sub / Apache Iggy**，并可选接入 **tsink 时序库** 与 **Walrus 分布式日志**；前端为**零依赖、可离线运行**的运营控制台。平台同时内置 **JT/T 808-2019 子集 TCP 网关** 与终端模拟器，形成从终端接入到业务闭环的完整链路。

![运营总览](docs/screenshots/dashboard.png)

> **适用边界**：本仓库是 TSP 参考实现/POC 基线，不是已通过检测的合规成品。JT/T 808 为 2019 子集；JT/T 809、GB/T 32960、JT/T 1078、GB/T 19056、等保/平台符合性检测尚未完成。不可直接用于监管备案或生产远程车控。详见 [REVIEW.md](docs/REVIEW.md) 与 [STANDARDS.md](docs/STANDARDS.md)。

## 1. 这不是一个车辆 CRUD Demo

平台围绕真实 TSP 的核心数据流实现了一个可运行的垂直切片：

| 能力域 | 已实现 |
| --- | --- |
| 车辆资产 | 车辆档案、VIN/车牌校验、状态机、终端绑定、组织与驾驶员 |
| 终端接入 | JT/T 808-2019 子集 TCP 网关、注册/鉴权/心跳/位置上报、7E 转义、BCD 终端号、XOR 校验 |
| 实时数据 | T-Box / HTTP 双通道写入、WebSocket 秒级推送、在线状态、SOC、里程、点火状态 |
| 行程引擎 | 点火自动开/闭行程、GPS 距离累计、平均/最高车速、轨迹回放 |
| 围栏引擎 | 圆形/多边形围栏、Haversine/射线法判定、进出状态机、进入/驶出告警 |
| 风险规则 | 超速、低电量、急减速、碰撞/胎压等事件模型，规则去抖与告警确认闭环 |
| 远程车控 | 上锁/解锁/启动/空调/寻车/充电等 12 类指令，状态流转与审计 |
| 运营看板 | 在线率、今日里程、告警分布、7 日趋势、指令成功率、车队健康度 |
| 安全与权限 | Argon2 密码、RS256 JWT/JWKS + refresh/吊销、admin/operator/viewer 三级 RBAC、组织域隔离（车辆/告警/指令/行程/看板/围栏）、操作审计 |
| 集群部署 | PostgreSQL/MySQL、Redis、API/网关多副本、Redis 选主、OpenRaft 控制面、K8s/HPA/PDB、Prometheus 指标 |
| 消息流 | 可选 Apache Iggy HTTP 事件日志、Walrus 持久事件归档 |
| 标准接入 | JT/T 809 / GB/T 32960 传输帧骨架、能力端点；业务消息体未实现 |
| 时序库 | 可选 tsink（Prometheus 兼容 HTTP）/ TDengine（REST SQL）轨迹热存储/查询 |
| 空间库 | PostgreSQL + PostGIS：轨迹/位置 geometry(Point,4326)、GiST 索引、附近/bbox 查询；无扩展时降级 lat/lon |
| 演示体系 | 18 台车辆、7 日历史、5 个围栏、告警/指令种子数据、内嵌车队模拟器 |

## 2. 快速启动

### 环境要求

- Rust 1.94.1+（SQLx 0.8；无需单独安装 SQLite/PostgreSQL 客户端库）
- Windows 需要 MSVC Build Tools；macOS 需要 Xcode CLT；Linux 需要 `build-essential`
- 无需 Node.js、数据库服务或外部 CDN

```bash
cd xingtu-tsp
cargo run
```

启动后访问：**http://127.0.0.1:8090**

| 账号 | 密码 | 角色 |
| --- | --- | --- |
| `admin` | `admin123` | 管理员（全部权限） |
| `operator` | `operator123` | 运营（业务写权限） |
| `viewer` | `viewer123` | 只读观察员 |

首次启动自动创建 SQLite 数据库 `data/tsp.db` 并写入演示车队。删除 `data/` 目录即可重置演示数据。

### Docker 运行

```bash
docker compose up --build
```

控制台：`http://127.0.0.1:8090`，JT/T 808 网关：`127.0.0.1:18080`，数据持久化在 `tsp-data` volume。

### 验证 JT/T 808 网关

内置模拟器会以种子终端 `013800000001`（车辆 沪A·D8F21）完成鉴权并连续上报 5 个位置点：

```bash
cargo run --bin jt808_sim
```

网关默认监听 `127.0.0.1:18080`。可自定义：

```bash
TSP_JT808_ADDR=127.0.0.1:18080 \
TSP_JT808_PHONE=013800000001 \
TSP_JT808_AUTH=XT000001 \
cargo run --bin jt808_sim
```

### 测试

```bash
cargo test
```

覆盖 JT/T 808 帧编解码/转义/校验/位置解析、批量定位、电子围栏几何算法、认证与 RBAC、位置上报、围栏进出、行程生成、告警、指令、审计、租户隔离及 809/32960 帧编解码；当前共 **97 个单元/数据层/消息/标准帧测试 + 11 个 API 集成测试**。

## 3. 界面

| 实时监控 | 电子围栏 |
| --- | --- |
| ![实时监控](docs/screenshots/live.png) | ![电子围栏](docs/screenshots/geofences.png) |

| 告警中心 | 指令中心 |
| --- | --- |
| ![告警中心](docs/screenshots/alerts.png) | ![指令中心](docs/screenshots/commands.png) |

控制台为纯原生 HTML/CSS/JavaScript，无构建步骤、无 CDN 依赖。地图是按经纬度投影绘制的**示意地图**（上海区域路网、黄浦江、区县标注），生产环境可替换为 MapLibre/高德/百度底图。

## 4. 系统架构

最新、可渲染的架构图（系统上下文、运行时容器、代码组件、遥测/指令/认证时序、部署拓扑）见 **[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)**。

当前主链路：

```text
T-Box ── JT/T 808 over TLS/mTLS ──> JT808 Gateway
                                        │ LocationSample
                                        ▼
Browser ── HTTPS / WebSocket ──> Axum API ──> services::telemetry
                                        │
                                        ▼
                         PostgreSQL 16 + PostGIS（权威数据）
                                        │
                       Redis（会话/租约/事件/jti 吊销）
                                        │
                              WebSocket 实时扇出
```

- 业务数据只写 PostgreSQL/PostGIS；SQLite/MySQL 是兼容/演示后端。
- Redis 只做短期协调与实时广播，不承载业务真相。
- Iggy/Walrus、tsink/TDengine、OpenRaft 均为可选组件，默认拓扑不依赖。
- JT808 支持 TLS/mTLS + CIDR 白名单；JWT 使用 RS256/JWKS + refresh 轮换 + jti 吊销。
- JT/T 808 是子集实现；JT/T 809、GB/T 32960 是帧/登录体骨架，不构成合规认证。

相关文档：

- [系统架构图](docs/ARCHITECTURE.md)
- [标准差距与实现路线](docs/STANDARDS_ROADMAP.md)
- [标准矩阵 CSV](docs/standards-matrix.csv)
- [标准版本台账](docs/STANDARDS_LEDGER.md)
- [合成一致性语料说明](docs/SIMULATION_SCOPE.md)
- [一致性测试入口](scripts/standards-test.ps1)
- [标准矩阵（历史版）](docs/STANDARDS.md)
- [安全加固](docs/SECURITY.md)
- [技术组件选型](docs/TECH_STACK.md)
- [代码结构](docs/STRUCTURE.md)
- [性能基准](docs/PERFORMANCE.md)
- [容量与慢查询](docs/CAPACITY.md)
- [10K/100K/1000K 基准](docs/BENCHMARK_REPORT.md)
- [PostGIS](docs/POSTGIS.md)
- [集群部署](docs/CLUSTER.md)

### 4.1 集群部署（PostgreSQL + Redis + Kubernetes）

集群模式把“接入、业务、实时、调度”拆成可水平扩展的角色：

- **API 无状态副本**：JWT 不落本地，可挂到任意 Pod；WebSocket 通过 Redis Pub/Sub 跨实例扇出。
- **JT/T 808 网关副本**：L4 LB 承接 TCP 长连接，终端重连可落到任意网关；Redis 记录终端会话归属。
- **分布式调度**：模拟器、离线巡检等单例任务通过 Redis 租约选主，避免多副本重复执行。
- **共享 PostgreSQL**：替换单机 SQLite，所有业务副本读写同一主库；轨迹层可继续接 TimescaleDB/ClickHouse。
- **Kubernetes 编排**：API/Gateway Deployment、HPA、PDB、Ingress、NetworkPolicy 与多 AZ 打散。

```bash
# 集群模式所需的核心环境变量
export TSP_CLUSTER_MODE=true
export TSP_DATABASE='postgres://tsp:***@postgres:5432/tsp?sslmode=disable'
export TSP_REDIS_URL='redis://redis:6379/0'
export TSP_NODE_ID='api-0'
```

完整的参考架构、容量模型、容灾方案与验收清单见 **[docs/CLUSTER.md](docs/CLUSTER.md)**，Kubernetes 清单见 **[deploy/k8s](deploy/k8s)**，部署步骤见 **[deploy/README.md](deploy/README.md)**。

本机有 Docker 时，可直接启动两 API + 两网关 + PostgreSQL + Redis 的集群演示：

```bash
docker compose -f deploy/docker-compose.cluster.yml up --build
```

控制台 `http://127.0.0.1:8090`，JT/T 808 接入 `127.0.0.1:18080`。

使用 Iggy 作为持久事件日志（Redis 仍负责选主/会话）：

```bash
docker compose -f deploy/docker-compose.cluster.yml -f deploy/docker-compose.iggy.yml up --build
```

或对已有服务设置：

```bash
export TSP_IGGY_URL=http://iggy:3000
export TSP_IGGY_USERNAME=iggy
export TSP_IGGY_PASSWORD=iggy
```

MySQL 单机/集群模式：

```bash
export TSP_DATABASE='mysql://tsp:password@127.0.0.1:3306/tsp'
export TSP_CLUSTER_MODE=false
cargo run
```

MySQL 适合已有 MySQL 基础设施的部署，也可在 `TSP_CLUSTER_MODE=true` 下与 Redis 组合做多副本；MySQL schema 使用 `VARCHAR` 与 `GET_LOCK` 适配，详见 [docs/CLUSTER.md](docs/CLUSTER.md)。

时序库与消息归档：

```bash
export TSP_TSINK_URL='http://tsink:9201'
export TSP_TSINK_TOKEN='optional-token'
export TSP_TDENGINE_URL='http://tdengine:6041'
export TSP_TDENGINE_USER='root'
export TSP_TDENGINE_PASSWORD='taosdata'
export TSP_TDENGINE_DATABASE='tsp'
export TSP_WALRUS_ADDR='walrus:8080'
export TSP_WALRUS_TOPIC='tsp-events'
```

- tsink 接收 Prometheus 文本写入，轨迹查询优先走 tsink，失败时自动回退关系库。
- TDengine 使用 taosAdapter REST SQL；自动创建数据库、`telemetry` 超级表与按车辆子表。
- Walrus 接收长度前缀 TCP 的 `REGISTER/PUT` 事件归档；由于 Walrus topic 使用共享 cursor，不承担 WebSocket 多节点广播，广播仍由 Redis/Iggy 负责。
- 查询优先级：tsink → TDengine → 关系库。

OpenRaft 控制面（三节点示例）：

```bash
export TSP_CLUSTER_MODE=true
export TSP_RAFT_ENABLED=true
export TSP_RAFT_NODE_ID=1
export TSP_RAFT_ADDR=127.0.0.1:7001
export TSP_RAFT_PEERS='1@127.0.0.1:7001,2@127.0.0.1:7002,3@127.0.0.1:7003'
export TSP_RAFT_BOOTSTRAP=true
```

Raft 只负责**调度器 Leader 选主与集群成员元数据**，不替换 PostgreSQL/MySQL 业务数据；Redis 继续负责网关会话和事件总线。关闭 `TSP_RAFT_ENABLED` 时仍使用原 Redis 租约选主。

![集群模式](docs/screenshots/dashboard-cluster.png)

## 5. 核心业务规则

### 5.1 位置上报与状态

- 同一套 `services::telemetry::ingest` 同时服务 HTTP、JT/T 808 和内嵌模拟器，避免多套逻辑。
- 速度 > 1 km/h 且点火 → `moving`；点火静止 → `idle`；充电状态保留 `charging`；无心跳 → `offline`。
- 每次位置都会更新 `vehicle_state` 并写入 `telemetry` 时序表。
- 轨迹接口返回**最近 N 个点**再按时间升序排列，适合实时回放。

### 5.2 行程自动切分

- 点火从 `false → true` 时创建 `active` 行程；
- 行程中按相邻 GPS 点 Haversine 距离累计里程，并更新最高车速；
- 熄火时写入结束时间、结束坐标、平均车速并置为 `completed`；
- 异常跳点（单段 > 5 km）不计入里程，避免 GPS 漂移污染。

### 5.3 电子围栏

- 圆形：Haversine 距离 ≤ 半径；
- 多边形：射线法（`point_in_polygon`）；
- 使用 `geofence_states` 记录每台车在每个围栏内的上一状态，仅状态变化时产生 `geofence_enter` / `geofence_exit`，避免每帧重复告警；
- 首次上报只建立基线，不产生进出告警。

### 5.4 告警规则

| 类型 | 触发条件 | 去抖 |
| --- | --- | --- |
| `overspeed` | 车速 > 120 km/h（>140 为 critical） | 5 分钟/车 |
| `low_soc` | 点火中 SOC ≤ 15%（<8 为 critical） | 30 分钟/车 |
| `harsh_brake` | 10 秒内减速 ≥ 20 km/h | 2 分钟/车 |
| `geofence_enter/exit` | 围栏状态翻转 | 状态机天然去重 |
| `collision` / `tire_pressure` / `device_offline` | 设备事件 / 运营事件模型 | 由接入方或巡检任务产生 |

### 5.5 远程指令

支持：`lock`、`unlock`、`start`、`stop`、`flash`、`honk`、`ac_on`、`ac_off`、`charge_start`、`charge_stop`、`window_close`、`find`。

当前实现的状态流转：`pending → sent → success / failed`。演示环境由后台任务模拟 T-Box 回执；生产接入时替换为 JT/T 808 下行消息（如 0x8300 文本下发、0x8500 车辆控制）并等待终端应答。

## 6. JT/T 808 网关

实现 JT/T 808-2019 的 **12 字节消息头子集**（非完整符合性实现）：

| 消息 | 方向 | 说明 |
| --- | --- | --- |
| `0x0100` 终端注册 | 上行 | 解析厂商/型号，生成/复用鉴权码，返回 `0x8100` |
| `0x0102` 终端鉴权 | 上行 | 校验终端号 + 鉴权码，返回 `0x8001` |
| `0x0002` 心跳 | 上行 | 刷新终端在线状态与最后心跳 |
| `0x0200` 位置信息汇报 | 上行 | 解析报警、状态、经纬度、高度、速度、方向、BCD 时间；支持 0x01 里程、0x31 卫星数扩展 |
| `0x8001` 平台通用应答 | 下行 | 请求流水号 + 结果 |
| `0x8100` 注册应答 | 下行 | 请求流水号 + 结果 + 鉴权码 |

协议细节：

- `0x7e` 帧定界；`0x7d 0x02` / `0x7d 0x01` 转义；
- 从消息头到消息体的 XOR 校验；
- 终端手机号 BCD[6]，12 位数字；
- 拒绝分包消息与加密消息（避免“半解析”造成脏数据）；
- 位置时间按北京时间（UTC+8）解析后统一存 UTC；
- 终端必须完成鉴权后才接受位置上报；未绑定车辆的终端返回失败。

> 安全边界：当前网关为明文 TCP，适用于内网/演示。生产部署应置于 VPN/专线之后，或增加 TLS、白名单、限流、终端证书与平台侧签名校验。

## 7. API 一览

完整接口与示例见 [docs/API.md](docs/API.md)。所有业务接口需要 `Authorization: Bearer <JWT>`。

```text
POST   /api/auth/login
GET    /api/auth/me

GET    /api/dashboard
GET    /api/vehicles
POST   /api/vehicles
GET    /api/vehicles/map
GET    /api/vehicles/nearby
GET    /api/vehicles/{id}
PATCH  /api/vehicles/{id}
DELETE /api/vehicles/{id}
GET    /api/vehicles/{id}/trajectory
GET    /api/vehicles/{id}/trips
GET    /api/vehicles/{id}/commands
POST   /api/vehicles/{id}/commands
POST   /api/vehicles/{id}/terminal
POST   /api/telemetry

GET    /api/trips
GET    /api/trips/{id}

GET    /api/geofences
POST   /api/geofences
GET    /api/geofences/{id}
PATCH  /api/geofences/{id}
DELETE /api/geofences/{id}

GET    /api/alerts
POST   /api/alerts/{id}/ack
POST   /api/alerts/ack-all

GET    /api/commands
GET    /api/commands/{id}
GET    /api/audit
GET    /api/audit/export

GET    /api/terminals
GET    /api/terminals/{id}
POST   /api/terminals/{id}/bind
POST   /api/terminals/{phone}/parameters
POST   /api/terminals/{phone}/control
POST   /api/terminals/{phone}/vehicle-control
POST   /api/terminals/{phone}/query-location
POST   /api/terminals/{phone}/query-parameters
GET    /api/terminals/{phone}/downlinks/{serial}

GET    /api/ws                     # WebSocket，Sec-WebSocket-Protocol: bearer.<JWT>
GET    /api/cluster/status       # 节点/选主/Redis 状态
GET    /api/metrics              # Prometheus 指标
GET    /api/health
```

错误统一返回：

```json
{ "error": { "code": "conflict", "message": "车辆当前离线或维护中，无法下发远程指令" } }
```

WebSocket 事件：`ready`、`telemetry`、`alert`、`command`、`fleet`、`resync`。

## 8. 配置

| 环境变量 | 默认值 | 说明 |
| --- | --- | --- |
| `TSP_BIND` | `127.0.0.1:8090` | HTTP 监听地址 |
| `TSP_DATABASE` | `data/tsp.db` | SQLite 路径，或 `postgres://` / `mysql://` URL |
| `TSP_WEB_DIR` | `web/` | 控制台静态资源目录 |
| `TSP_JWT_PRIVATE_KEY_FILE` | 空 | RS256 私钥 PEM 文件；生产必填，集群多节点共用 |
| `TSP_JWT_PUBLIC_KEY_FILE` | 空 | RS256 公钥 PEM；省略时从私钥派生 |
| `TSP_JWT_KEY_ID` | `tsp-rs256-1` | JWKS `kid` |
| `TSP_JWT_ACCESS_TTL_MINUTES` | `15` | access token 有效期 |
| `TSP_JWT_REFRESH_TTL_HOURS` | `168` | refresh token 有效期 |
| `TSP_JWT_PREVIOUS_KEYS` | 空 | 旧公钥保留窗口，格式 `kid=path,kid=path` |
| `TSP_JT808_ADDR` | `127.0.0.1:18080` | JT/T 808 监听地址 |
| `TSP_JT808_TLS_CERT` | 空 | JT808 TLS 证书 PEM；生产建议启用 |
| `TSP_JT808_TLS_KEY` | 空 | JT808 TLS 私钥 PEM |
| `TSP_JT808_ALLOW_PLAINTEXT` | `false` | 无证书时是否允许明文 TCP；生产默认拒绝 |
| `TSP_JT808_CLIENT_CA` | 空 | mTLS 客户端 CA PEM；配置后强制终端证书 |
| `TSP_JT808_ALLOWED_CIDRS` | 空 | 终端 CIDR 白名单，逗号分隔 |
| `TSP_CLUSTER_MODE` | `false` | 集群模式：启用 Redis 协调，数据库需为 PostgreSQL/MySQL |
| `TSP_REDIS_URL` | 空 | Redis URL，集群模式必填 |
| `TSP_REDIS_ALLOW_PLAINTEXT` | `false` | 生产集群是否允许 `redis://`；默认只接受 `rediss://` |
| `TSP_IGGY_URL` | 空 | 可选 Iggy HTTP 地址，如 `http://iggy:3000` |
| `TSP_IGGY_STREAM` | `xingtu-tsp` | Iggy stream 名称 |
| `TSP_IGGY_TOPIC` | `events` | Iggy topic 名称 |
| `TSP_IGGY_USERNAME` | `iggy` | Iggy 用户名 |
| `TSP_IGGY_PASSWORD` | `iggy` | Iggy 密码 |
| `TSP_TSINK_URL` | 空 | 可选 tsink HTTP 地址，如 `http://tsink:9201`；轨迹查询优先走 tsink |
| `TSP_TSINK_TOKEN` | 空 | tsink Bearer Token |
| `TSP_TDENGINE_URL` | 空 | TDengine taosAdapter 地址，如 `http://tdengine:6041` |
| `TSP_TDENGINE_USER` | `root` | TDengine 用户名 |
| `TSP_TDENGINE_PASSWORD` | `taosdata` | TDengine 密码 |
| `TSP_TDENGINE_DATABASE` | `tsp` | TDengine 数据库名 |
| `TSP_WALRUS_ADDR` | 空 | 可选 Walrus TCP 地址，如 `127.0.0.1:8080` |
| `TSP_WALRUS_TOPIC` | `tsp-events` | Walrus 归档 topic |
| `TSP_RAFT_ENABLED` | `false` | 启用 OpenRaft 控制面（选主/集群元数据） |
| `TSP_RAFT_NODE_ID` | 由 `TSP_NODE_ID` 稳定哈希 | OpenRaft 节点 ID（u64，多副本必须唯一） |
| `TSP_RAFT_ADDR` | `127.0.0.1:7001` | OpenRaft RPC 监听地址 |
| `TSP_RAFT_PEERS` | 空 | 静态成员列表：`1@host:7001,2@host:7002` |
| `TSP_RAFT_BOOTSTRAP` | `true` | 是否用静态成员初始化 Raft 集群 |
| `TSP_RAFT_DATA_DIR` | `./raft-data/<node>` | Raft log/vote/committed + 状态机 JSON 持久化目录 |
| `TSP_POSTGIS_ENABLED` | `true` | PostgreSQL 可用 PostGIS 时启用 geometry/GiST；失败自动降级 |
| `TSP_ENV` | `development` | `production` 时强制校验 secret/demo/默认密码/指令模式 |
| `TSP_COMMAND_MODE` | 演示 `simulated`，否则 `disabled` | `simulated` / `jt808` / `disabled` |
| `TSP_CORS_ORIGINS` | 空 | 允许的浏览器 Origin，逗号分隔；空为同源 |
| `TSP_ADMIN_PASSWORD` | 空 | 生产空库首次启动的管理员密码 |
| `TSP_NODE_ID` | 主机名/UUID | 节点唯一标识 |
| `TSP_NODE_ROLE` | `api` | 节点角色：`api` / `gateway` |
| `TSP_DEMO_MODE` | `true` | 是否启用内嵌车队模拟器 |
| `RUST_LOG` | `xingtu_tsp=info` | 日志级别 |

生产模式最小配置：

```bash
export TSP_ENV=production
export TSP_DEMO_MODE=false
export TSP_COMMAND_MODE=disabled
export TSP_JWT_PRIVATE_KEY_FILE=/etc/tsp/jwt-private.pem
export TSP_JWT_PUBLIC_KEY_FILE=/etc/tsp/jwt-public.pem
export TSP_REDIS_URL='rediss://:password@redis:6379/0'
export TSP_JT808_TLS_CERT=/etc/tsp/jt808-cert.pem
export TSP_JT808_TLS_KEY=/etc/tsp/jt808-key.pem
export TSP_ADMIN_PASSWORD='首次启动的管理员长密码'
export TSP_CORS_ORIGINS='https://tsp.example.com'
```

生产模式会拒绝无 RS256 私钥、`redis://` 明文 Redis、无 JT808 TLS/显式明文开关、默认 admin/admin123、demo 数据、模拟指令；空库首启必须提供 `TSP_ADMIN_PASSWORD`。WebSocket 不再接受 URL query token，改用 `Sec-WebSocket-Protocol: bearer.<JWT>`。

示例：

```bash
TSP_BIND=0.0.0.0:8090 \
TSP_JWT_PRIVATE_KEY_FILE='/etc/tsp/jwt-private.pem' \
TSP_DEMO_MODE=false \
cargo run --release
```

## 9. 数据模型

`users`、`vehicles`、`vehicle_state`、`terminals`、`telemetry`、`trips`、`geofences`、`geofence_states`、`alerts`、`commands`、`audit_logs` 共 11 张核心表。

- 时间统一存储为 UTC RFC3339 字符串；
- `telemetry(vehicle_id, ts)` 与 `alerts(acknowledged, ts)` 建有复合索引；
- 启动时清理 7 天前轨迹，防止演示库无限增长；
- SQLite 开启 WAL 与外键约束；PostgreSQL 使用事务级 advisory lock 串行化多副本建表，使用会话级 advisory lock 串行化 demo seed。
- MySQL 使用 InnoDB/utf8mb4；`sqlx::Any` 将 MySQL `TEXT` 识别为 BLOB，因此 schema 使用有界 `VARCHAR`（payload/polygon 8000，message/result 2048），超长文本需改表或走对象存储。终端号唯一性使用生成列，MariaDB 需 10.2+。
- MySQL 集群使用 `GET_LOCK` 串行化首次种子；DDL 使用 `CREATE TABLE IF NOT EXISTS`，建议生产升级仍由独立 migration Job 执行。
- 当前没有版本化 ALTER 迁移；生产升级建议用独立 migration Job，遵循 expand/contract。

## 10. 生产化演进建议

当前版本定位为**可运行的 TSP 垂直切片**，已覆盖接入、计算、告警、车控、运营闭环。面向规模化商用，建议：

1. **存储**：业务库已支持 PostgreSQL；下一步将位置/轨迹迁移至 TimescaleDB 或 ClickHouse，Redis 缓存在线状态与围栏状态。
2. **接入层**：Kafka / MQTT 承接高并发终端数据；JT/T 808 网关水平扩展并按终端号分片。
3. **协议**：补齐 808 分包、0x0704 批量位置、0x8300/0x8500 下行、OTA 升级、GB/T 32960 新能源国标。
4. **规则引擎**：将阈值、去抖、围栏策略配置化；支持静默期、分级升级、通知渠道（短信/企微/电话）。
5. **多租户**：`tenant_id` 隔离、组织树、数据权限、SSO/OIDC。
6. **安全**：TLS/mTLS、终端证书、接口限流、敏感指令二次确认、密钥托管、审计归档。
7. **可观测性**：Prometheus 指标、OpenTelemetry 链路、终端接入质量看板、告警。
8. **地图**：接入 MapLibre + 高德/百度/OSM 瓦片，增加聚合、热力、轨迹纠偏与路网匹配。

## 11. 说明

- 地图为离线示意地图，不依赖第三方瓦片服务，坐标与底图仅用于演示。
- 内嵌模拟器仅用于前端动态演示，可通过 `TSP_DEMO_MODE=false` 关闭；关闭后仍可用 `jt808_sim` 或 `POST /api/telemetry` 注入数据。
- 文本与界面为中文，代码注释为英文，便于二次开发。
- 项目以 MIT 许可发布，可自由用于学习与原型验证。
- 集群模式已在本地用 PostgreSQL 16 + Redis 验证：双节点跨实例 WebSocket 扇出、会话注册、Leader 故障切换均可工作。
- OpenRaft 已用本机 3 节点验证：成员 1/2/3 选举、Leader 统一、kill Leader 后 term 递增并重新选主。
- MySQL 已用 MariaDB 11.4 验证完整应用启动、18 台车辆 seed、Dashboard API 与双节点集群。
- tsink 已用真实 `tsink-server 0.10.2` 验证 Prometheus 写入、`/api/v1/query_range` 查询与轨迹合并。
- TDengine 适配器已用 taosAdapter REST 契约 mock 验证建库/超级表/写入/查询；生产需连接真实 TDengine 3.x。
- Walrus 协议已从上游源码核实：命令与响应均为 4 字节小端长度前缀；上游 distributed-walrus 在 Windows 上因存储后端限制无法启动，实际运行需 Linux 环境。
- Iggy HTTP 适配器已按 Iggy REST 契约用本地 mock 验证登录、Stream/Topic 创建、生产与消费。

## Object storage

RustFS is the S3-compatible object store for recorded media. With `TSP_MEDIA_UDP_ADDR` and `TSP_RUSTFS_ARCHIVE=true`, 1078 media is segmented on keyframes/time and uploaded as raw H.264/AAC. Archive objects are listed/downloaded via `/api/media/archive`. Archive playback is available at `/api/media/archive/playback`; run `scripts/rustfs-smoke.ps1` for a real S3 upload/download check. See [docs/RUSTFS.md](docs/RUSTFS.md) for Compose/Kubernetes, credentials, and the MinIO migration path.

---

**星途车联 XINGTU TSP** · Rust / Axum / Tokio / SQLx / SQLite + PostgreSQL + MySQL / Redis / Apache Iggy / JT/T 808 / WebSocket
