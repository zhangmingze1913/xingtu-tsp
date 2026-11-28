# 全量测试报告（2026-11-28）

## 1. 结果

```text
lib                    302 passed
integration            48 passed
doc tests               0 passed
all-targets            通过
fmt                    clean
clippy --all-targets   clean
```

## 2. 测试范围

### 单元/协议

```text
301 + 1 新增
cluster::tests::disabled_terminal_cache_is_fail_open
standards catalog
JT808 codec/control/gateway
JT809 frame/business/cipher
JT1078 pipeline/reorder
GB32960 data units/signature/activation
GB19056
PII vault
```

### 集成

```text
tests/auth.rs
tests/vehicles.rs
tests/standards.rs
tests/storage.rs
tests/cluster.rs
tests/standards_conformance.rs
tests/jt808_terminal_sim.rs
tests/jt809_sim.rs
tests/jt809_tls_sim.rs
tests/gb32960_sim.rs
tests/jt1078_sim.rs
tests/pii_vault.rs
tests/standards_fuzz.rs
tests/health.rs
tests/postgis_health.rs（opt-in，未设置 env 时跳过）
```

## 3. 本轮测试发现/覆盖

新增：

```text
disabled_terminal_cache_is_fail_open
```

验证 Redis 不可用/disabled 时：

```text
cached_terminal -> None
invalidate -> no-op
```

无 panic，继续查 PostgreSQL。

## 4. 未执行项

| 项目 | 原因 | 需要 |
| --- | --- | --- |
| PostGIS 实际查询 | 当前无 Docker/PostGIS 环境 | `TSP_TEST_POSTGIS_URL` |
| Redis 容器缓存命中 | 当前无 Redis 服务 | `TSP_TEST_REDIS_URL` |
| Docker/K8s 全链路 | 当前 Docker CLI 不可用 | Docker Desktop + kind |
| 1M 连接 | 无百万级压测机 | 分布式压测 |
| 火焰图符号 | Rust PDB 符号化失败 | 可用 PDB/符号服务 |

这些不是通过，不能写成通过。

## 5. 测试结论

```text
功能/协议/安全单测通过
SQLite/HTTP/JT808 集成通过
fmt/clippy 通过
```

仍需：

```text
Redis 终端缓存命中/失效测试
PostGIS bbox/nearby 复测
Docker/kind 全链路复测
1M 连接阶梯压测
```

## 6. 建议

1. 在 CI 增加 `TSP_TEST_REDIS_URL` 可选 job。
2. 在 CI 增加 PostGIS service job。
3. 缓存命中指标纳入 SLO。
4. 1M 压测单独环境，不在开发机执行。
5. 不把 opt-in 跳过计为通过。


## 7. Kiri 补充

Kiri 0.2.2 已安装并运行 WSL2 engine：

```text
docker context kiri
HTTP+JT808 full-chain: 403.6 req/s, 0 failed
container RSS: 60.09 MiB
```

生产 runtime 为 Tokio；可选实验 runtime 依赖已卸载。
