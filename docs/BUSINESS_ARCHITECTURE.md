# 星途 TSP 业务架构

## 1. 定位

星途 TSP 面向道路运输/车队运营场景，提供：

```text
车辆资产
终端接入
实时位置
轨迹/行程
电子围栏
告警处置
远程车控
媒体归档
运营看板
标准/监管数据交换
```

当前是参考实现/POC 基线，不是已通过监管备案或第三方标准检测的成品。

## 2. 业务参与者

```mermaid
flowchart LR
    ADMIN[平台管理员] --> CONSOLE[运营控制台]
    OPS[车队运营] --> CONSOLE
    VIEWER[只读分析] --> CONSOLE
    DRIVER[驾驶员] --> TERM[T-Box/终端]
    TERM --> GW[JT/T 808 网关]
    CONSOLE --> API[TSP API]
    GW --> API
    REG[上级监管平台] -. JT/T 809 .-> API
    MEDIA[车载视频终端] -. JT/T 1078 .-> API
```

| 角色 | 主要诉求 |
| --- | --- |
| 平台管理员 | 组织、账号、终端、系统配置、审计 |
| 车队运营 | 车辆、围栏、告警、指令、行程 |
| 只读分析 | 看板、轨迹、报表 |
| 驾驶员/终端 | 注册、鉴权、位置/告警上报、接收指令 |
| 监管平台 | 平台数据交换、报警/监管请求 |
| 视频终端 | 实时/历史视频、资源上传 |

## 3. 业务能力地图

```mermaid
flowchart TB
    subgraph Access["接入"]
        A1[终端注册/鉴权]
        A2[位置/心跳/告警]
        A3[视频信令/媒体]
        A4[平台间交换]
    end

    subgraph Fleet["车队运营"]
        F1[车辆档案]
        F2[终端绑定]
        F3[驾驶员/PII]
        F4[组织隔离]
    end

    subgraph RealTime["实时运营"]
        R1[实时位置]
        R2[在线状态]
        R3[WebSocket 推送]
        R4[地图/附近]
    end

    subgraph Analysis["分析"]
        S1[轨迹]
        S2[行程]
        S3[里程/能耗]
        S4[围栏进出]
    end

    subgraph Control["控制"]
        C1[告警确认]
        C2[远控指令]
        C3[参数下发]
        C4[媒体上传/回放]
    end

    subgraph Governance["治理"]
        G1[RBAC/审计]
        G2[数据删除/导出]
        G3[对账/监管导出]
        G4[标准符合性证据]
    end

    Access --> Fleet
    Fleet --> RealTime
    RealTime --> Analysis
    Analysis --> Control
    Control --> Governance
```

## 4. 核心业务流程

### 4.1 车辆与终端入网

```mermaid
sequenceDiagram
    participant O as 运营
    participant A as TSP API
    participant D as 数据库
    participant T as 终端

    O->>A: 创建车辆
    A->>D: vehicles
    O->>A: 绑定终端 IMEI
    A->>D: terminals
    T->>A: 0x0100 注册
    A-->>T: 0x8100 + auth code
    T->>A: 0x0102 鉴权
    A-->>T: 通用应答
    T->>A: 0x0002 心跳
    A->>D: 在线状态
```

### 4.2 实时位置与告警

```mermaid
flowchart LR
    T[终端] -->|0x0200| G[JT808 网关]
    G --> C[Telemetry 核心]
    C --> DB[(业务库)]
    C --> WS[WebSocket]
    C -. 可选 .-> TS[(TDengine/tsink)]
    DB --> DASH[运营总览]
    DB --> ALERT[告警中心]
```

### 4.3 围栏与告警处置

```mermaid
flowchart TB
    P[位置点] --> F[围栏判定]
    F -->|进入/驶出| A[生成告警]
    A --> Q[告警中心]
    Q --> ACK[运营确认]
    ACK --> AUDIT[审计]
    A --> EXP[CSV/加密导出]
```

### 4.4 远控指令

```mermaid
sequenceDiagram
    participant O as 运营
    participant A as API
    participant D as DB
    participant G as JT808 网关
    participant T as 终端

    O->>A: 创建指令
    A->>D: commands(pending)
    A->>G: 下行帧
    G->>T: 0x8500/0x8103 等
    T-->>G: 应答
    G->>D: 状态更新
    O->>A: 查询结果
```

### 4.5 视频与归档

```mermaid
flowchart LR
    C[1078 终端] --> M[媒体 Hub]
    M --> R[(RustFS)]
    M --> P[实时播放]
    R --> H[历史回放]
    R --> S[spool/重试]
```

### 4.6 对账与监管导出

```mermaid
flowchart LR
    PG[(PostgreSQL/Kingbase)] --> CMP[对账]
    TD[(TDengine)] --> CMP
    CMP --> H[对账历史]
    H --> J[导出任务]
    J --> E[CSV/AES-GCM]
    E --> T[单次下载令牌]
```

## 5. 业务对象

```mermaid
erDiagram
    ORGANIZATION ||--o{ USER : contains
    ORGANIZATION ||--o{ VEHICLE : owns
    VEHICLE ||--o| TERMINAL : binds
    VEHICLE ||--o{ DRIVER : assigns
    VEHICLE ||--o{ TRIP : has
    VEHICLE ||--o{ ALERT : raises
    VEHICLE ||--o{ COMMAND : receives
    VEHICLE ||--o{ GEOFENCE_STATE : inside
    GEOFENCE ||--o{ GEOFENCE_STATE : tracks
    VEHICLE ||--o{ MEDIA_ARCHIVE : records
```

当前模型用 `org_name` 做组织隔离，尚未实现独立组织表。

## 6. 权限模型

| 能力 | admin | operator | viewer |
| --- | --- | --- | --- |
| 组织范围 | 全部 | 本组织 | 本组织 |
| 车辆写 | 是 | 是 | 否 |
| 告警确认 | 是 | 是 | 否 |
| 远控指令 | 是 | 是 | 否 |
| 终端管理 | 是 | 是 | 否 |
| PII 删除 | 是 | 否 | 否 |
| 对账/导出 | 是 | 本组织 | 否 |
| 审计 | 是 | 否 | 否 |

## 7. 业务指标

```text
车辆在线率
终端在线率
今日里程
告警数/级别
指令成功率
围栏进出次数
轨迹完整率
对账差异数
PII 删除请求数
```

## 8. 业务边界

- 不替代真实终端/平台联调。
- 不替代监管备案。
- 不替代第三方标准检测。
- 不承诺所有地方/团体标准自动适用。
- 视频/远控的实际效果依赖终端和网络。

## 9. 相关文档

- [ARCHITECTURE_DIAGRAMS.md](ARCHITECTURE_DIAGRAMS.md)
- [TECHNICAL_SOLUTION.md](TECHNICAL_SOLUTION.md)
- [OPERATIONS_ARCHITECTURE.md](OPERATIONS_ARCHITECTURE.md)
- [DEPLOYMENT_ARCHITECTURE.md](DEPLOYMENT_ARCHITECTURE.md)
