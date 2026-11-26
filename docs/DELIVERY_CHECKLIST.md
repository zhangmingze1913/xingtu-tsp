# 交付验收清单

> 生产就绪状态与退出标准见 [PRODUCTION_READINESS.md](PRODUCTION_READINESS.md)。

## 1. 代码与构建

- [x] `cargo fmt --check`
- [x] `cargo clippy --all-targets -- -D warnings`
- [x] `cargo test --lib`：299 passed
- [x] 集成/协议测试：45 passed
- [x] `cargo build --release`
- [x] Git 工作区干净
- [x] 版本与提交记录可追溯

## 2. 数据库

### PostgreSQL

- [x] 真实连接
- [x] 迁移
- [x] 跨连接池告警 claim
- [x] PostGIS 3.4 扩展
- [x] geometry + GiST + geography 索引
- [x] nearby/bbox 空间查询
- [x] 对账导出任务
- [ ] 生产规模 `EXPLAIN` 调优
- [x] 流复制 smoke
- [x] 只读从库
- [x] 手工 promote
- [ ] 自动 failover/生产切换

### KingbaseES

- [x] V8R6 容器启动
- [x] V9 容器启动
- [x] `DB_MODE=pg` 连接
- [x] 迁移/seed/车辆/轨迹/告警
- [x] lat/lon 降级 nearby
- [ ] KGIS 扩展
- [ ] geometry/GiST 查询
- [ ] 官方授权镜像与 License
- [ ] 主备/高可用

### TDengine

- [ ] 真实容器/集群
- [ ] 双写/查询/对账
- [ ] spool 重放
- [ ] 规模压测

### MySQL

- [x] MySQL 8.0 容器启动
- [x] schema/migrations
- [x] demo seed
- [x] 车辆/轨迹/告警 API
- [x] 重跑幂等索引
- [x] 环境变量门控测试
- [ ] MySQL 8.4/5.7 版本矩阵

### SQLite

- [x] 单元/集成测试

## 3. 存储与媒体

- [ ] RustFS 真实桶
- [ ] 1078 实时流
- [ ] 归档上传/下载
- [ ] 转封装播放
- [ ] 保留策略
- [ ] spool 持久卷

## 3.1 可选存储/中间件微测试

- [x] Iggy HTTP mock
- [x] Walrus TCP mock
- [x] tsink HTTP mock
- [x] Redis 真实 session（门控）
- [x] RustFS S3 mock
- [x] TDengine REST mock
- [x] Vault mock
- [ ] 真实 Iggy/Walrus/tsink/Redis 集群
- [ ] 真实 RustFS/TDengine

## 4. 安全

- [x] Argon2 密码
- [x] JWT RS256/JWKS
- [x] refresh token 轮换
- [x] RBAC/组织隔离
- [x] PII 加密
- [x] 导出 AES-256-GCM
- [x] 下载令牌单次消费
- [ ] 生产 TLS/mTLS 证书
- [ ] Redis TLS/AUTH
- [ ] Vault/KMS 实际接入
- [ ] 渗透测试
- [ ] 等保/密码测评

## 5. 可观测性

- [x] `/api/health`
- [x] `/api/ready`
- [x] `/api/metrics`
- [x] Grafana 仪表板 JSON
- [ ] Prometheus 实际抓取
- [ ] 告警规则接入 Alertmanager
- [ ] 日志集中化
- [ ] 链路追踪

## 6. 部署

- [x] k3d 完整链路（PG+RustFS+TLS）
- [x] 单机 Compose
- [x] 集群 Compose
- [x] K8s 清单
- [x] Kingbase 容器模板
- [x] 备份/恢复脚本
- [x] release preflight
- [ ] 生产 Ingress/TLS
- [ ] HPA/PDB 实测
- [ ] 灾备演练

## 7. 文档

- [x] 使用手册
- [x] 技术文档
- [x] 运维手册
- [x] 架构文档
- [x] 性能评估
- [x] 使用建议
- [x] 安全文档
- [x] Kingbase/KGIS 文档
- [x] TDengine/RustFS 专题
- [x] API 文档
- [x] 标准边界说明

## 8. 真实环境验证矩阵

| 项目 | 状态 | 证据 |
| --- | --- | --- |
| 本机 Docker | 通过 | Docker 29.9.3 |
| Kingbase V8R6 | 通过 | 容器 + API |
| Kingbase V9 | 通过 | 容器 + health |
| PostgreSQL | 通过 | 真实连接/导出任务 |
| KGIS | 未验证 | 镜像无插件 |
| TDengine | 未验证 | 无实例 |
| RustFS | 未验证 | 无实例 |
| Alertmanager | 未验证 | 无实例 |
| Vault | 未验证 | mock only |
| 真实终端 | 未验证 | 无设备 |
| 弱网 | 未验证 | 未模拟 |
| 百万级 | 历史数据 | 当前未复测 |

## 9. 验收结论模板

```text
代码测试：通过
本地真实 DB：PostgreSQL/Kingbase 关系型通过
空间扩展：未通过（KGIS 缺失）
时序库：未验证
对象存储：未验证
标准合规：未检测
生产就绪：否，需目标环境验证
```

## 10. 签字

| 角色 | 姓名 | 日期 | 结论 |
| --- | --- | --- | --- |
| 开发 |  |  |  |
| 测试 |  |  |  |
| 运维 |  |  |  |
| 安全 |  |  |  |
| 业务 |  |  |  |

> 未完成项不得用“默认支持”“理论上可以”替代真实验证记录。
