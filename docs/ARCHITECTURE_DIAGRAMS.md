# 星途 TSP 架构图集

本文件用 Mermaid 描述当前代码的真实结构。图中“可选”表示有配置开关或可降级，不代表已通过生产验证。

## 1. 系统上下文

```mermaid
flowchart LR
    TB[T-Box / 车机] -->|JT/T 808 TLS/mTLS| GW[JT808 Gateway]
    OPS[运营控制台] -->|HTTPS / WebSocket| API[Axum API]
    GW -->|LocationSample| CORE[Telemetry / Command Core]
    API --> CORE
    CORE --> PG[(PostgreSQL / KingbaseES)]
    CORE --> REDIS[(Redis)]
    CORE -. optional .-> TSDB[TDengine / tsink]
    CORE -. optional .-> S3[(RustFS)]
    CORE -. optional .-> BUS[Iggy / Walrus]
    RAFT[OpenRaft control plane] -. metadata .-> API
    PROM[Prometheus / Grafana] -. scrape .-> API
```

边界：

- 终端不直接访问数据库。
- 浏览器只访问 API、静态资源和 WebSocket。
- Redis 不保存业务真相。
- TDengine/tsink/RustFS/Iggy/Walrus 都是可选组件，可降级或明确失败。
- OpenRaft 只管理控制面。

## 2. 运行时容器

```mermaid
flowchart TB
    subgraph Edge["接入层"]
        LB4[L4 LoadBalancer]
        LB7[Ingress / L7]
        GW1[JT808 Gateway]
        API1[API Pod]
        API2[API Pod]
    end

    subgraph Core["应用核心"]
        AUTH[Auth / RBAC]
        HTTP[HTTP Handlers]
        WS[WebSocket Hub]
        TELE[Telemetry Ingest]
        CMD[Command Dispatcher]
        MAINT[Maintenance Workers]
        SIM[Demo Simulator]
    end

    subgraph Data["数据层"]
        PG[(PostgreSQL/KingbaseES)]
        REDIS[(Redis)]
        RAFT[(OpenRaft)]
        TSDB[(TDengine/tsink)]
        S3[(RustFS)]
    end

    TB[T-Box] --> LB4 --> GW1
    OPS[Browser] --> LB7 --> API1
    LB7 --> API2
    GW1 --> TELE
    API1 --> AUTH --> HTTP
    API2 --> AUTH
    HTTP --> TELE
    HTTP --> CMD
    HTTP --> WS
    TELE --> PG
    CMD --> PG
    AUTH --> REDIS
    WS --> REDIS
    TELE -. optional .-> TSDB
    TELE -. optional .-> S3
    MAINT --> PG
    MAINT --> TSDB
    SIM --> TELE
    RAFT --> REDIS
```

## 3. 代码模块依赖

```mermaid
flowchart TB
    API[api/] --> SVC[services/]
    API --> MEDIA[media/]
    API --> SEC[security/]
    SVC --> DB[db/]
    SVC --> TS[timeseries.rs]
    SVC --> TD[tdengine/]
    JT[jt808/] --> SVC
    STD[standards/] --> JT
    MEDIA --> S3[(RustFS)]
    DB --> PG[(PG/KES/MySQL/SQLite)]
    TD --> TDS[(TDengine)]
    MAINT[services/maintenance.rs] --> DB
    MAINT --> TD
    MAINT --> MEDIA
```

规则：

- `db` 不依赖 `api`。
- 协议层不直接写 SQL。
- handler 不做无限循环或长事务。
- 外部组件失败不得阻塞核心业务事务。

## 4. 定位入库时序

```mermaid
sequenceDiagram
    participant T as T-Box
    participant G as JT808 Gateway
    participant S as Telemetry Service
    participant D as Database
    participant C as Dashboard Cache
    participant W as WebSocket
    participant TS as TDengine

    T->>G: 0x0200 location
    G->>G: decode / auth / dedup
    G->>S: LocationSample
    S->>D: BEGIN
    S->>D: telemetry + vehicle_state
    S->>D: trips + geofence_states
    S->>D: alerts
    S->>D: COMMIT
    S->>C: clear org cache
    S->>W: broadcast
    S-->>TS: enqueue (optional)
```

关键点：

- 业务写入在同一事务。
- 缓存清理和 WebSocket 在提交后。
- TDengine 是异步副本，失败不影响主事务。

## 5. 认证与 RBAC

```mermaid
sequenceDiagram
    participant U as User
    participant A as Auth API
    participant R as Redis
    participant H as Handler
    participant D as DB

    U->>A: POST /api/auth/login
    A->>D: load user + Argon2 verify
    A->>R: store refresh jti
    A-->>U: access_token + refresh_token
    U->>H: Authorization Bearer
    H->>H: verify RS256 + jti + role
    H->>H: org scope check
    H->>D: query with org filter
    D-->>H: rows
    H-->>U: JSON
```

## 6. TDengine 双写与对账

```mermaid
flowchart LR
    ING[Ingest] --> PG[(PostgreSQL)]
    ING --> Q[Bounded Queue]
    Q --> W[TDengine Writer]
    W -->|ok| TD[(TDengine)]
    W -->|fail| SPOOL[Local Spool]
    SPOOL -->|replay| W
    SCHED[Reconciler] --> PG
    SCHED --> TD
    SCHED -->|mismatch| HIST[(History)]
    HIST -->|webhook| AM[Alertmanager]
    HIST -->|export job| CSV[CSV/AES-GCM]
```

## 7. RustFS 录制归档

```mermaid
sequenceDiagram
    participant C as JT1078 Client
    participant M as Media Hub
    participant R as RustFS
    participant S as Spool

    C->>M: UDP/RTP H.264/AAC
    M->>M: keyframe-aligned segment
    M->>R: PutObject
    alt upload failed
        M->>S: save pending
        S->>R: replay later
    end
    M-->>C: playback fMP4 on demand
```

## 8. Kubernetes 部署

```mermaid
flowchart TB
    subgraph K8s["Kubernetes"]
        ING[Ingress] --> API1[API Pod]
        ING --> API2[API Pod]
        GW[JT808 Gateway Pod]
        API1 --> PG[(PostgreSQL)]
        API2 --> PG
        API1 --> REDIS[(Redis)]
        GW --> REDIS
        API1 -. optional .-> RUSTFS[(RustFS)]
        API1 -. optional .-> TD[(TDengine)]
        API1 --> PVC[(Export PVC)]
    end
    TB[T-Box] --> GW
    OPS[Browser] --> ING
```

## 9. 数据模型

```mermaid
erDiagram
    USERS ||--o{ AUDIT_LOGS : writes
    VEHICLES ||--o| VEHICLE_STATE : has
    VEHICLES ||--o{ TELEMETRY : reports
    VEHICLES ||--o{ TRIPS : takes
    VEHICLES ||--o{ ALERTS : raises
    VEHICLES ||--o{ COMMANDS : receives
    VEHICLES ||--o| TERMINALS : binds
    GEOFENCES ||--o{ GEOFENCE_STATES : tracks
    VEHICLES ||--o{ GEOFENCE_STATES : inside

    USERS { text id PK text username text role text org_name }
    VEHICLES { text id PK text vin text plate_no text org_name }
    TELEMETRY { bigint id PK text vehicle_id text ts float lat float lon geometry geom }
    ALERTS { text id PK text vehicle_id text alert_type text level text ts int acknowledged }
    COMMANDS { text id PK text vehicle_id text status text command }
```

## 10. 可选组件状态

| 组件 | 默认 | 失败行为 |
| --- | --- | --- |
| PostGIS/KGIS | 尝试启用 | 降级 lat/lon |
| TDengine | 关闭 | `tdengine` 模式启动失败；`dual` 降级 |
| tsink | 关闭 | 查询回退 PG |
| RustFS | 关闭 | 不归档，spool 不启用 |
| Redis | 单机可关闭 | 本地 leader / 本地广播 |
| Iggy/Walrus | 关闭 | 退回 Redis/本地 |

## 11. 相关文档

- [ARCHITECTURE.md](ARCHITECTURE.md)
- [TECHNICAL_SOLUTION.md](TECHNICAL_SOLUTION.md)
- [OPERATIONS_GUIDE.md](OPERATIONS_GUIDE.md)
- [USER_GUIDE.md](USER_GUIDE.md)
