# 使用与选型建议

## 1. 按规模选择

| 在线车辆 | 推荐数据库 | 时序 | 部署 | 说明 |
| ---: | --- | --- | --- | --- |
| < 500 | PostgreSQL + PostGIS | PG | 单机/2 副本 | 最简单 |
| 500–10K | PostgreSQL + PostGIS | PG 或 `dual` | 3 API + 2 网关 | 常规生产 |
| 10K–100K | PostgreSQL/KingbaseES | TDengine `dual` | 多副本 + 独立 DB | 轨迹查询分离 |
| > 100K | KingbaseES/PostgreSQL + TDengine | TDengine | 分片/多集群 | 需要容量测试 |

信创要求：

```text
KingbaseES V8R6/V9 + KGIS
```

注意：

- 必须 `DB_MODE=pg`
- 必须安装 KGIS 才能使用 geometry/GiST
- 无 KGIS 时只能 lat/lon 降级

## 2. 不要这样用

1. 不要把 SQLite 当生产主库。
2. 不要让应用在没有 KGIS 的情况下承诺空间查询性能。
3. 不要在没有 TDengine 的环境启用 `TSP_TIMESERIES_BACKEND=tdengine`；启动会失败。
4. 不要在没有 RustFS 的环境承诺历史视频。
5. 不要用 `admin/admin123` 上生产。
6. 不要把 `/api/metrics`、导出、审计接口暴露到公网。
7. 不要用 `debug` 构建做容量结论。
8. 不要把“测试通过”写成“标准合规”。

## 3. 推荐上线顺序

```text
1. 单机 SQLite + demo 验证功能
2. PostgreSQL + PostGIS 验证业务
3. 开启 JWT 密钥、TLS/mTLS、备份
4. 集群 API/网关 + Redis
5. 按需启用 RustFS 归档
6. 按需启用 TDengine dual
7. 信创替换 KingbaseES + KGIS
8. 真实终端/弱网/规模压测
```

每一步都要有回滚点。

## 4. 数据保留建议

| 数据 | 建议 |
| --- | --- |
| telemetry | 热数据 30–90 天，冷数据归档/降采样 |
| alerts | 180–365 天 |
| audit_logs | 按合规要求，通常 ≥ 1 年 |
| pii_vault | 与业务数据同生命周期，删除需审计 |
| RustFS 媒体 | 7–90 天，按事故取证需求 |
| 对账历史 | 90 天 |
| 导出任务 | 24 小时 |

## 5. 安全建议

- RS256 私钥离线保存，公钥通过 JWKS 发布
- JWT access token 短 TTL，refresh token 轮换
- JT808 生产启用 TLS/mTLS
- Redis 启用 AUTH/TLS
- PII 密钥与导出密钥分离
- 导出文件强制加密
- 下载令牌单次消费
- 管理员操作审计
- 定期轮换密钥

## 6. 性能建议

- 列表分页 ≤ 200
- 轨迹查询必须带时间范围
- dashboard 保留 2s 缓存
- 大导出用后台任务
- 数据库连接池按 CPU/IO 调整，不要盲目 ×N
- 轨迹规模大时用 TDengine，不要把所有查询压到 PG
- KGIS 可用后为 geom 建 GiST 索引

## 7. 运维建议

- 每日备份 + 每周恢复演练
- 监控 `/api/ready`、错误率、DB 连接数、磁盘
- 监控 TDengine queue/spool/alerts
- 监控 RustFS 上传失败与 spool
- 升级前预发跑迁移
- 保留 `.manifest.json` 校验备份

## 8. 验收建议

必须区分：

```text
代码测试通过
真实数据库连接
真实终端联调
第三方标准检测
生产容量测试
```

当前状态：

- 代码测试通过
- PostgreSQL 真实连接通过
- KingbaseES V8R6/V9 关系型通过
- KGIS 未通过
- TDengine/RustFS 未验证
- 真实终端/弱网未验证

不要把前两项外推为后四项。

## 9. 适用结论

本系统适合：

- TSP 参考实现/POC
- 车队运营原型
- 信创数据库适配验证
- 协议/数据链路联调

不适合直接：

- 监管合规备案
- 无人值守生产远控
- 未经压测的大规模生产
- 替代第三方标准检测
