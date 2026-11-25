# 星途 TSP 文档中心

本目录是交付文档的唯一入口。历史评审文档保留在 `docs/` 中，但以本页列出的文档为当前基线。

## 1. 建议阅读顺序

| 角色 | 先读 | 再读 |
| --- | --- | --- |
| 产品/运营 | [USER_GUIDE.md](USER_GUIDE.md) | [RECOMMENDATIONS.md](RECOMMENDATIONS.md) |
| 开发 | [TECHNICAL_GUIDE.md](TECHNICAL_GUIDE.md) | [ARCHITECTURE.md](ARCHITECTURE.md) |
| 运维/SRE | [OPERATIONS_GUIDE.md](OPERATIONS_GUIDE.md) | [SECURITY.md](SECURITY.md) |
| 交付/验收 | [DELIVERY_CHECKLIST.md](DELIVERY_CHECKLIST.md) | [PERFORMANCE_EVALUATION_2026-11-25.md](PERFORMANCE_EVALUATION_2026-11-25.md) |
| 信创 | [KINGBASE_KGIS.md](KINGBASE_KGIS.md) | [OPERATIONS_GUIDE.md](OPERATIONS_GUIDE.md) |
| 时序库 | [TIMESERIES_BACKENDS.md](TIMESERIES_BACKENDS.md) | [TIMESERIES_OPERATIONS.md](TIMESERIES_OPERATIONS.md) |

## 2. 核心交付文档

```text
USER_GUIDE.md                          使用手册
TECHNICAL_GUIDE.md                     技术文档
OPERATIONS_GUIDE.md                    运维手册
ARCHITECTURE.md                        架构文档
PERFORMANCE_EVALUATION_2026-11-25.md   本次性能评估
RECOMMENDATIONS.md                     选型与使用建议
DELIVERY_CHECKLIST.md                  交付/验收清单
```

## 3. 专题文档

```text
API.md                     HTTP API
SECURITY.md                安全与密钥
CLUSTER.md                 集群与 Raft
POSTGIS.md                 空间数据
KINGBASE_KGIS.md           信创 KingbaseES/KGIS
TIMESERIES_BACKENDS.md     TDengine 选型
TIMESERIES_OPERATIONS.md   TDengine 运维
RUSTFS.md                  录制/归档存储
INTEGRATIONS.md            外部集成
STANDARDS.md               标准实现边界
SIMULATION_SCOPE.md        模拟器范围
```

## 4. 历史文档

以下文档保留历史评审与阶段结论，不再作为当前实现说明：

```text
MASTER_REVIEW_2026-11-23.md
FINAL_STATUS_2026-11-23.md
REVIEW.md
CODE_REVIEW.md
STRUCTURE_PERFORMANCE_REVIEW.md
STANDARDS_REASSESSMENT_2026-11-21.md
```

## 5. 当前交付状态

| 项目 | 状态 |
| --- | --- |
| 单元/协议测试 | 291 passed |
| 集成测试 | 32 passed |
| fmt / clippy | clean |
| PostgreSQL 真实连接 | 已验证 |
| KingbaseES V8R6/V9 关系型 | 已验证（容器） |
| KGIS 空间扩展 | 未验证（镜像不含插件） |
| TDengine | 未验证（无实例） |
| RustFS | 未验证（无实例） |
| 百万级压测 | 历史报告有数据，当前未复测 |

> 不要把“代码通过测试”表述为“标准合规”或“生产就绪”。标准边界见 [STANDARDS.md](STANDARDS.md) 和 [SIMULATION_SCOPE.md](SIMULATION_SCOPE.md)。
