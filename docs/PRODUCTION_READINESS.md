> 最新业务/技术 Go/No-Go 评分见 [PRODUCTION_GO_NO_GO_2026-11-26.md](PRODUCTION_GO_NO_GO_2026-11-26.md)。

# 生产就绪评估与退出计划

> 本文件是“生产就绪”章节的正式口径。**当前不满足生产就绪**，不能把下列部分通过项外推为生产可用。

## 1. 结论

```text
GitHub 提交：符合
参考实现/POC：可用
生产就绪：未完成
标准合规：未检测
```

已通过：

- 代码测试、真实容器单节点验证
- PostgreSQL 主从流复制、手工 promote、Patroni 自动 failover
- PostGIS 10 万/100 万空间基准
- K8s SQLite smoke + k3d 完整链路

未通过：

- K8s 多副本/托管/生产网络
- KGIS
- 多节点 Iggy/tsink（Walrus 双节点已通过）
- 真实终端/弱网
- 第三方标准检测

## 2. 能力矩阵

| 能力 | 状态 | 证据/缺口 |
| --- | --- | --- |
| 单机 SQLite | 通过 | 299 单元测试 |
| PostgreSQL 16 | 通过 | 真实容器 |
| PostGIS 3.4 | 通过 | geometry/GiST/bbox |
| PostgreSQL 主从 | 通过（手工） | streaming/async/lag0/promote |
| PostgreSQL 自动 failover | 通过（Patroni） | 3 节点，pg1 停止后 pg3 自动升主并写入 |
| MySQL 8 | 通过 | 容器 + API |
| KingbaseES 关系型 | 通过 | V8R6/V9 容器 |
| KGIS | 未通过 | 镜像无插件/授权 |
| TDengine 单节点 | 通过 | REST smoke + API |
| TDengine 集群 | 未验证 | 需多节点 |
| RustFS 单节点 | 通过 | S3 建桶/上传 |
| RustFS 多节点/生产 | 未验证 | 需 Helm/多盘 |
| Redis 单节点 | 通过 | session |
| Redis HA/TLS | 未验证 | 需集群/证书 |
| Vault dev | 通过 | KV 同步 |
| Vault 生产 | 未验证 | 需 HA/Unseal |
| Iggy 单节点 | 通过 | HTTP h2c |
| Iggy 集群 | 未验证 | 需多节点 |
| Walrus 单节点 | 通过 | REGISTER/PUT/GET |
| Walrus 双节点 | 通过 | 节点 1 PUT、节点 2 GET 成功 |
| tsink 单节点 | 通过 | Prometheus import |
| tsink 集群 | 未验证 | 需多节点 |
| K8s smoke | 通过 | Pod/健康/登录/车辆 |
| K8s 完整链路（单节点 k3d） | 通过 | PG+RustFS+TLS+API+重启持久化 |
| K8s 多副本/托管生产 | 未通过 | 需云 K8s/托管 PG/Redis |
| 真实终端 | 未验证 | 无设备 |
| 弱网 | 未验证 | 无网络模拟 |
| 标准合规 | 未检测 | 第三方 |

## 3. 本轮新增尝试

### 3.1 K8s 完整链路

目标：

```text
K8s + PostgreSQL + RustFS + JWT/JT808 TLS Secret + 应用
```

**k3d 结果：通过**

```text
k3d v5.8.3 / k3s v1.31.5
postgres-0 1/1 Running
redis      1/1 Running
rustfs     1/1 Running
tsp-api    1/1 Running
database_engine=postgres
vehicles total=18
JT808 TLS enabled
RustFS archive enabled
删除 API Pod 重建后 vehicles 仍为 18
```

完整步骤见 [KUBERNETES_SMOKE.md](KUBERNETES_SMOKE.md)。

**kind 结果：失败**

```text
kind v0.33.0
kubeadm init wait-control-plane 失败
client rate limiter Wait ... context deadline
```

已尝试清理/重建/`--retain`/释放资源；判断为当前 Docker Desktop/WSL 的 kind 控制面初始化问题。k3d 已完成同一清单的应用层验证。

### 3.2 Patroni 自动 failover

目标：

```text
etcd + 3 x Patroni/PostgreSQL 16
```

结果：**通过**

```text
pg1 Leader / pg2 Replica streaming / pg3 Replica streaming
stop pg1
pg3 Leader (TL 2)
INSERT failover_probe -> 1 row
pg2 pg_is_in_recovery = t
```

配置与步骤见 [POSTGRES_HA.md](POSTGRES_HA.md)。

### 3.3 Walrus 双节点

目标：

```text
walrus-n1 / walrus-n2，验证 leader forwarding
```

结果：**通过**

```text
node1 REGISTER OK
node2 REGISTER OK
node1 PUT OK
node2 GET OK hello-cluster
```

说明：第一次用 compose + 源码挂载失败（磁盘/mount），改用镜像直接启动两个节点后通过。

## 4. 生产退出标准

只有全部满足才可称为“生产就绪”：

```text
[ ] 目标环境压测：10K/100K/1M，含并发/长稳
[x] Patroni 自动 failover 原语（单环境 smoke）
[ ] RPO/RTO/多 AZ/客户端切换演练
[x] K8s 单节点 PG/Redis/RustFS/TLS 完整链路（k3d）
[ ] K8s 多副本/托管 PG/Redis、Ingress TLS、滚动升级/回滚
[ ] KGIS 真实扩展与空间查询
[ ] 多节点 Iggy/Walrus/tsink 故障切换
[ ] 真实终端联调：注册/鉴权/位置/远控/断网重连
[ ] 弱网/丢包/乱序/重复报文测试
[ ] 安全渗透/密钥管理/等保差距评估
[ ] 第三方标准符合性检测
[ ] 备份恢复/PITR/灾备演练
[ ] 监控告警/SLO/值班手册
[ ] 许可证/第三方组件合规审查
```

## 5. 分阶段计划

### P0（阻断生产）

| 任务 | 验收 |
| --- | --- |
| PostgreSQL 自动 failover | Patroni/云 RDS，主库故障自动切，RPO≤5s |
| K8s 完整链路 | 单节点已通过；仍需多副本/托管/Ingress TLS |
| 真实终端联调 | 808 注册/鉴权/位置/远控/重连 |
| 安全评估 | 渗透/密钥/权限/审计 |

### P1（规模与信创）

| 任务 | 验收 |
| --- | --- |
| KGIS | 官方插件，geometry/GiST/bbox |
| 多节点 Iggy/Walrus/tsink | 节点故障不丢事件 |
| 100 万级并发/长稳 | p95/错误率/资源达标 |
| 备份/PITR/灾备 | 恢复演练通过 |

### P2（合规与运营）

| 任务 | 验收 |
| --- | --- |
| 标准检测 | 第三方报告 |
| SLO/值班 | 告警闭环 |
| 容量模型 | 按车辆数/轨迹量估算 |
| 许可证审查 | 第三方组件合规 |

## 6. 当前建议

1. 不要把本仓库直接用于监管备案/生产远控。
2. 先在目标云/信创环境做 **P0**，不要在开发机继续堆容器。
3. PostgreSQL 自动 failover 用 Patroni + etcd 或云 RDS，不要手写脚本。
4. K8s 完整链路优先用云 K8s/托管 PG/RustFS，kind 只做清单 smoke。
5. KGIS 必须拿到官方插件包与 License。
6. 真实终端/弱网必须用硬件或专业网络模拟器。

## 7. 相关文档

- [DELIVERY_CHECKLIST.md](DELIVERY_CHECKLIST.md)
- [OPERATIONS_GUIDE.md](OPERATIONS_GUIDE.md)
- [POSTGRES_HA.md](POSTGRES_HA.md)
- [CONTAINER_TESTS.md](CONTAINER_TESTS.md)
- [PERFORMANCE_EVALUATION_2026-11-25.md](PERFORMANCE_EVALUATION_2026-11-25.md)
