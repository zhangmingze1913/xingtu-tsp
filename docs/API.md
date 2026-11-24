# 星途 TSP · HTTP / WebSocket API

Base URL：`http://127.0.0.1:8090/api`

除 `POST /auth/login`、`GET /health` 外，全部接口要求：

```http
Authorization: Bearer <JWT>
Content-Type: application/json
```

角色：

| 角色 | 读 | 车辆/围栏/告警/指令写 | 删除车辆/围栏 |
| --- | --- | --- | --- |
| `admin` | ✓ | ✓ | ✓ |
| `operator` | ✓ | ✓ | ✗ |
| `viewer` | ✓ | ✗ | ✗ |

错误格式：

```json
{
  "error": {
    "code": "conflict",
    "message": "车辆当前离线或维护中，无法下发远程指令"
  }
}
```

常见状态码：`400` 参数、`401` 未登录/Token 失效、`403` 权限不足、`404` 资源不存在、`409` 业务冲突、`422` JSON 解析失败、`500` 服务端异常。

## 1. 认证

### POST /auth/login

```json
{ "username": "admin", "password": "admin123" }
```

`200`：

```json
{
  "token": "eyJhbGciOiJIUzI1NiIs...",
  "user": {
    "id": "b2a3...",
    "username": "admin",
    "display_name": "张伟",
    "role": "admin",
    "created_at": "2026-11-15T12:15:52Z"
  }
}
```

### GET /auth/me

返回当前登录用户。

## 2. 运营看板

### GET /dashboard

```json
{
  "fleet": {
    "total": 18, "online": 14, "moving": 11, "idle": 2, "charging": 1,
    "offline": 3, "maintenance": 1,
    "online_rate": 77.8, "avg_soc": 57.1,
    "total_mileage_km": 506020.5, "terminal_online_rate": 77.8
  },
  "alerts": { "active": 6, "critical": 3, "warning": 2, "today": 10 },
  "mileage_7d": [{ "label": "11-09", "value": 117.1 }],
  "alert_trend_7d": [{ "label": "11-09", "value": 4 }],
  "status_distribution": [{ "name": "moving", "value": 11 }],
  "recent_alerts": [],
  "command": { "total_7d": 10, "success_rate": 80.0 },
  "generated_at": "2026-11-15T12:55:35Z"
}
```

## 3. 车辆

### GET /vehicles

Query：

| 参数 | 类型 | 说明 |
| --- | --- | --- |
| `search` | string | 车牌 / VIN / 驾驶员模糊搜索 |
| `status` | string | `moving` / `idle` / `charging` / `offline` / `maintenance` / `online` |
| `page` | int | 默认 1 |
| `page_size` | int | 默认 20，最大 200 |

`200`：

```json
{
  "items": [
    {
      "id": "veh_001",
      "vin": "LSVAA2A41NX000001",
      "plate_no": "沪A·D8F21",
      "model": "ID.4 CROZZ",
      "color": "冰川白",
      "energy_type": "EV",
      "driver_name": "张伟",
      "driver_phone": "138****2101",
      "org_name": "星途出行 · 浦东车队",
      "created_at": "2026-07-18T12:15:52Z",
      "state": {
        "vehicle_id": "veh_001",
        "lat": 31.2322, "lon": 121.4763,
        "speed": 45.1, "heading": 42.0,
        "ignition": true, "soc": 77.7, "odometer_km": 12450.4,
        "status": "moving", "updated_at": "2026-11-15T12:57:05Z"
      },
      "terminal": {
        "id": "term_001", "imei": "861234567890001", "phone": "013800000001",
        "iccid": "89860012345678900001", "model": "XT-TBOX-5G",
        "firmware": "V1.4.2", "protocol": "JT/T 808-2019",
        "vehicle_id": "veh_001", "status": "online",
        "last_heartbeat_at": "2026-11-15T12:57:05Z",
        "created_at": "2026-07-18T12:15:52Z", "bound_at": "2026-08-07T12:15:52Z"
      }
    }
  ],
  "total": 18, "page": 1, "page_size": 20
}
```

### POST /vehicles

`admin` / `operator`。成功返回 `201`。

```json
{
  "vin": "LSVAA2A41NX123456",
  "plate_no": "沪A·D12345",
  "model": "ID.4 CROZZ",
  "color": "冰川白",
  "energy_type": "EV",
  "driver_name": "张伟",
  "driver_phone": "13800000000",
  "org_name": "星途出行 · 浦东车队"
}
```

VIN 必须 17 位；`energy_type` 取 `EV` / `HEV` / `PHEV` / `ICE`。

### PATCH /vehicles/{id}

可更新：`plate_no`、`model`、`color`、`energy_type`、`driver_name`、`driver_phone`、`org_name`。未传字段保持不变。

### DELETE /vehicles/{id}

仅 `admin`。级联删除车辆状态、终端绑定、轨迹、行程、告警、指令。

### GET /vehicles/map?status=moving

轻量地图数据：

```json
[{ "id":"veh_001","plate_no":"沪A·D8F21","model":"ID.4 CROZZ","lat":31.2322,"lon":121.4763,
   "speed":45.1,"heading":42.0,"status":"moving","soc":77.7,"ignition":true,"odometer_km":12450.4 }]
```

### GET /vehicles/{id}/trajectory

Query：`from`、`to`（RFC3339）、`limit`（默认 1000，最大 2000）。

返回最近 N 个点并按时间升序：

```json
[{
  "id": 1024, "vehicle_id": "veh_001", "ts": "2026-11-15T12:57:05Z",
  "lat": 31.2322, "lon": 121.4763, "speed": 45.1, "heading": 42.0,
  "ignition": true, "soc": 77.7, "odometer_km": 12450.4, "source": "jt808"
}]
```

### POST /telemetry

T-Box / 模拟器 / 第三方平台的位置注入接口，与 JT/T 808 位置上报走同一业务链路。

```json
{
  "vehicle_id": "veh_001",
  "ts": "2026-11-15T12:57:05Z",
  "lat": 31.2304,
  "lon": 121.4737,
  "speed": 45.0,
  "heading": 90,
  "ignition": true,
  "soc": 78.0,
  "odometer_km": 12450.5,
  "source": "api"
}
```

`ts` 可省略（默认当前 UTC）。成功 `201` 返回 `IngestOutcome`：

```json
{
  "vehicle_id": "veh_001", "plate_no": "沪A·D8F21",
  "ts": "2026-11-15T12:57:05Z", "lat": 31.2304, "lon": 121.4737,
  "speed": 45.0, "heading": 90.0, "ignition": true, "soc": 78.0,
  "odometer_km": 12450.5, "status": "moving",
  "alerts": [], "trip_started": false, "trip_ended": false
}
```

## 4. 电子围栏

### GET /geofences

```json
[{
  "id": "geo_zhangjiang", "name": "张江科学城", "kind": "circle",
  "center_lat": 31.205, "center_lon": 121.599, "radius_m": 2600,
  "polygon": null, "color": "#22d3ee", "active": true,
  "created_at": "2026-09-16T12:15:52Z", "inside_count": 1
}]
```

`polygon` 坐标顺序为 `[lon, lat]`，如 `[[121.82,30.86],[121.98,30.86],[122.0,30.96]]`。

### POST /geofences

圆形：

```json
{ "name": "张江科学城", "kind": "circle", "center_lat": 31.205, "center_lon": 121.599,
  "radius_m": 2600, "color": "#22d3ee", "active": true }
```

多边形：

```json
{ "name": "临港新片区", "kind": "polygon",
  "polygon": [[121.82,30.86],[121.98,30.86],[122.0,30.96],[121.86,30.99],[121.8,30.93]],
  "color": "#8b5cf6", "active": true }
```

半径范围 50–50000 米；多边形至少 3 个顶点。

### PATCH /geofences/{id}

```json
{ "name": "新名称", "radius_m": 3000, "color": "#f59e0b", "active": false }
```

### DELETE /geofences/{id}

仅 `admin`。

## 5. 告警

### GET /alerts

Query：`status=active|acknowledged`、`level=critical|warning|info`、`vehicle_id`、`page`、`page_size`。

```json
{
  "items": [{
    "id": "0b4c...", "vehicle_id": "veh_005", "alert_type": "low_soc",
    "level": "critical", "title": "动力电池电量过低",
    "message": "车辆 苏E·D6P52 剩余电量 12%，续航不足 60 km，请立即规划补能",
    "lat": 31.229, "lon": 121.454, "ts": "2026-11-15T12:43:52Z",
    "acknowledged": false, "ack_by": null, "ack_at": null,
    "plate_no": "苏E·D6P52", "model": "理想 L7"
  }],
  "total": 6, "page": 1, "page_size": 10
}
```

### POST /alerts/{id}/ack

```json
{ "note": "已联系驾驶员" }
```

返回更新后的告警。重复确认幂等。

### POST /alerts/ack-all

```json
{ "acknowledged": 6 }
```

## 6. 指令

### POST /vehicles/{id}/commands

`admin` / `operator`。成功返回 `202`。

```json
{ "command": "ac_on", "payload": null }
```

可选指令：

`lock` `unlock` `start` `stop` `flash` `honk` `ac_on` `ac_off` `charge_start` `charge_stop` `window_close` `find`

状态：

- `pending` 已创建、待下发
- `sent` 已送达 T-Box
- `success` 执行成功
- `failed` 执行失败
- `cancelled` 已取消

离线/维保车辆返回 `409`。

### GET /commands

Query：`status`、`vehicle_id`、`page`、`page_size`。

### GET /commands/{id}

返回单条指令及执行结果。

## 7. 终端

### GET /terminals

返回终端列表及绑定车牌：

```json
[{
  "id": "term_001", "imei": "861234567890001", "phone": "013800000001",
  "iccid": "89860012345678900001", "model": "XT-TBOX-5G", "firmware": "V1.4.2",
  "protocol": "JT/T 808-2019", "vehicle_id": "veh_001", "status": "online",
  "last_heartbeat_at": "2026-11-15T12:57:05Z", "created_at": "...",
  "bound_at": "...", "plate_no": "沪A·D8F21"
}]
```

`auth_code` 不会通过 API 序列化返回。

### POST /terminals/{id}/bind

```json
{ "vehicle_id": "veh_001" }
```

传 `{"vehicle_id": null}` 解绑。

### POST /vehicles/{id}/terminal

创建并绑定新终端：

```json
{
  "imei": "861234567890019",
  "phone": "013800000019",
  "iccid": "89860012345678900019",
  "model": "XT-TBOX-5G"
}
```

## 8. 行程

### GET /trips

Query：`vehicle_id`、`page`、`page_size`。

```json
{
  "items": [{
    "id": "b6f2...", "vehicle_id": "veh_001",
    "start_ts": "2026-11-15T12:36:52Z", "end_ts": null,
    "distance_km": 7.8, "avg_speed": 28.4, "max_speed": 58.0,
    "start_lat": 31.2397, "start_lon": 121.4998,
    "end_lat": 31.2322, "end_lon": 121.4763,
    "status": "active", "plate_no": "沪A·D8F21", "model": "ID.4 CROZZ"
  }],
  "total": 40, "page": 1, "page_size": 20
}
```

### GET /trips/{id}

返回 `{ "trip": {...}, "points": [...] }`，用于轨迹回放。

## 9. WebSocket

```
ws://127.0.0.1:8090/api/ws?token=<JWT>
```

连接后首先收到：

```json
{ "type": "ready", "username": "admin", "server_time": "2026-11-15T12:57:05Z" }
```

### telemetry

```json
{
  "type": "telemetry", "vehicle_id": "veh_001", "plate_no": "沪A·D8F21",
  "ts": "2026-11-15T12:57:05Z", "lat": 31.2304, "lon": 121.4737,
  "speed": 45.0, "heading": 90.0, "ignition": true,
  "soc": 78.0, "odometer_km": 12450.5, "status": "moving"
}
```

### alert

```json
{ "type": "alert", "alert": { "...": "同 Alert 对象" } }
```

### command

```json
{ "type": "command", "command": { "...": "同 Command 对象" } }
```

### fleet

```json
{ "type": "fleet", "online": 14, "moving": 11, "ts": "2026-11-15T12:57:05Z" }
```

### resync

事件总线积压时发送，客户端应重新拉取当前页面数据：

```json
{ "type": "resync", "skipped": 12 }
```

## 10. 健康检查

### GET /health

`200` 健康 / `503` 数据库不可用：

```json
{
  "status": "ok",
  "version": "0.1.0",
  "database": true,
  "cluster_enabled": true,
  "cluster_leader": false,
  "timestamp": "2026-11-15T12:57:05Z"
}
```

### GET /metrics

Prometheus 文本格式，公开供集群采集。包含 HTTP 请求数、位置入库数、告警数、指令数、WebSocket/网关连接数、选主状态：

```text
xingtu_http_requests_total 128
xingtu_telemetry_ingested_total 90312
xingtu_ws_connections 17
xingtu_jt808_connections 1204
xingtu_cluster_leader 1
```

### GET /cluster/status

需要登录。返回当前节点的 Redis 协调状态：

```json
{
  "enabled": true,
  "node_id": "api-0",
  "leader": true,
  "redis_connected": true,
  "event_bus": "redis",
  "raft_enabled": true,
  "raft_node_id": 1,
  "raft_leader": 1,
  "raft_term": 3,
  "raft_state": "Leader",
  "raft_last_log": 4
}
```

`event_bus` 为 `redis` / `iggy` / `local`；`raft_*` 仅在 `TSP_RAFT_ENABLED=true` 时有意义。

## 11. 快速联调（PowerShell）

```powershell
$login = Invoke-RestMethod -Method Post http://127.0.0.1:8090/api/auth/login `
  -ContentType application/json -Body '{"username":"admin","password":"admin123"}'
$headers = @{ Authorization = "Bearer $($login.token)" }
Invoke-RestMethod http://127.0.0.1:8090/api/dashboard -Headers $headers

Invoke-RestMethod -Method Post http://127.0.0.1:8090/api/telemetry -Headers $headers `
  -ContentType application/json -Body '{"vehicle_id":"veh_001","lat":31.23,"lon":121.47,"speed":50,"ignition":true,"soc":70}'
```

## 12. cURL

```bash
TOKEN=$(curl -s http://127.0.0.1:8090/api/auth/login \
  -H 'Content-Type: application/json' \
  -d '{"username":"admin","password":"admin123"}' | jq -r .token)

curl -s http://127.0.0.1:8090/api/vehicles?page_size=5 \
  -H "Authorization: Bearer $TOKEN" | jq

curl -s -X POST http://127.0.0.1:8090/api/vehicles/veh_001/commands \
  -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  -d '{"command":"find"}' | jq
```

## 媒体归档（RustFS）

```http
GET /media/archive?sim=138000000001&channel=1
GET /media/archive/playback?key=media/138000000001/1/2026/11/24/080001-000000.h264
GET /media/archive/{key}
```

- 列表返回 `prefix` 与 `objects[{key,size}]`
- 回放下载同名 H.264/AAC，服务端 remux 为 fMP4
- 下载返回 raw 对象，`Content-Type` 为 `video/h264` 或 `audio/aac`
- `key` 必须带 `TSP_RUSTFS_PREFIX`，拒绝 `..` 与反斜杠

