# 星途 TSP 使用手册

适用对象：运营人员、车队管理员、只读分析人员。本文只描述当前代码实际提供的能力。

## 1. 登录与角色

演示数据账号：

| 账号 | 密码 | 角色 | 能力 |
| --- | --- | --- | --- |
| `admin` | `admin123` | 管理员 | 全部组织、全部写操作 |
| `operator` | `operator123` | 运营 | 本组织写操作 |
| `viewer` | `viewer123` | 只读 | 本组织只读 |

生产环境不要保留默认密码。首次生产启动需设置 `TSP_ADMIN_PASSWORD`。

浏览器打开 `TSP_BIND` 对应地址，例如 `http://127.0.0.1:8090`。

## 2. 控制台导航

| 页面 | 用途 |
| --- | --- |
| 运营总览 | 在线率、今日里程、告警、7 日趋势、指令成功率 |
| 实时监控 | 车辆位置、状态、WebSocket 实时刷新 |
| 车辆管理 | 车辆档案、增删改、CSV 导出、轨迹入口 |
| 终端管理 | JT/T 808 终端、绑定、参数、状态 |
| 行程分析 | 点火自动开/关行程、里程、均速、最高速 |
| 电子围栏 | 圆形/多边形围栏、进入/驶出告警 |
| 告警中心 | 超速、低电量、围栏、碰撞等告警与确认 |
| 指令中心 | 上锁/解锁/启动/空调/寻车/充电等远控 |
| 媒体播放 | JT/T 1078 实时/历史、归档播放 |

## 3. 典型流程

### 3.1 新建车辆并绑定终端

1. 打开 **车辆管理** → 新建。
2. 填写 VIN、车牌、车型、组织、驾驶员。
3. 打开 **终端管理**，录入 IMEI/手机号并绑定车辆。
4. 终端上线后，总览与实时监控出现该车。

### 3.2 查看实时位置

- **实时监控** 页面通过 WebSocket 更新。
- 也可用 API：

```http
GET /api/vehicles?page=1&page_size=50
Authorization: Bearer <access_token>
```

### 3.3 查看轨迹

1. 车辆详情 → 轨迹。
2. 选择时间范围。
3. 可导出 CSV：

```http
GET /api/vehicles/{id}/trajectory/export
```

### 3.4 处理告警

1. 打开 **告警中心**。
2. 按级别/车辆筛选。
3. 确认告警，记录处理人。
4. 可导出 CSV。

### 3.5 下发远控指令

1. 打开 **指令中心**。
2. 选择车辆与指令类型。
3. 查看状态：`pending` → `sent` → `completed` / `failed`。
4. 指令走 JT/T 808 下行；终端离线时不会假装成功。

### 3.6 媒体归档

- **媒体播放** 页面可查看实时流与归档列表。
- 归档来自 RustFS；未配置 RustFS 时该页面无历史数据。
- 归档对象是原始 H.264/AAC，播放接口按需转封装为 fMP4。

## 4. 常用 API

### 登录

```http
POST /api/auth/login
Content-Type: application/json

{"username":"admin","password":"admin123"}
```

返回 `access_token`、`refresh_token`。后续请求：

```http
Authorization: Bearer <access_token>
```

### 上报定位

```http
POST /api/telemetry
Content-Type: application/json

{
  "vehicle_id": "veh_001",
  "lat": 31.2304,
  "lon": 121.4737,
  "speed": 45.0,
  "heading": 90.0,
  "ignition": true,
  "soc": 80.0,
  "odometer_km": 100.0
}
```

返回告警列表、行程状态、围栏状态。

### 附近车辆

```http
GET /api/vehicles/nearby?lat=31.23&lon=121.47&radius_m=5000
```

`radius_m` 范围 10–50000。PostGIS/KGIS 可用时走空间查询；否则用 Haversine 降级。

### 地图 bbox

```http
GET /api/vehicles/map?bbox=121.2,31.0,121.8,31.4
```

### 对账历史

```http
GET /api/integrations/timeseries/reconciliations?limit=50
```

管理员可查全部组织；非管理员只能查本组织。

### 导出对账历史

```http
GET /api/integrations/timeseries/reconciliations/export?footer=true
```

大导出建议用任务：

```http
POST /api/integrations/timeseries/reconciliations/export/jobs
{"max_rows":5000}
```

然后轮询任务并下载。

## 5. 使用建议

1. 演示模式 `TSP_DEMO_MODE=true` 只用于演示/测试，生产关闭。
2. 生产必须修改 `admin` 密码并配置独立 JWT/PII/导出密钥。
3. 先用 PostgreSQL；信创环境用 KingbaseES（`DB_MODE=pg`），大型轨迹再评估 TDengine。
4. 车辆数超过单机 SQLite 容量前迁到 PostgreSQL/KingbaseES。
5. 归档媒体依赖 RustFS；不配置时不要向用户承诺历史视频。
6. 对账/导出任务有上限，按组织分批执行。
7. 任何“标准合规/生产就绪”结论都需要第三方检测，不能由本控制台代替。

## 6. 已知边界

- JT/T 808 是 2019 子集，不是全量协议。
- JT/T 809、GB/T 32960、JT/T 1078、GB/T 19056 只有部分消息/帧结构。
- KingbaseES 容器已做关系型验证，KGIS 插件未验证。
- TDengine、RustFS、Alertmanager、Vault 的真实环境验证取决于部署。
- 远控指令的实际车辆执行依赖真实终端与网络。

标准边界见 [STANDARDS.md](STANDARDS.md) 与 [SIMULATION_SCOPE.md](SIMULATION_SCOPE.md)。
