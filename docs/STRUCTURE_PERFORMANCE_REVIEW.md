# 代码结构与性能复查（2026-11-23）

## 1. 模块规模

最大文件：

| 文件 | 行数 | 主要问题 |
| --- | ---: | --- |
| `src/standards/gb32960.rs` | 2884 | 编解码、密码、监测状态机、测试混在一个文件 |
| `src/jt808/control.rs` | 2559 | 多类控制消息编解码集中 |
| `src/standards/jt809.rs` | 2377 | 帧、加密、业务体、序列号、测试集中 |
| `src/standards/jt1078.rs` | 901 | 协议与媒体封装边界模糊 |
| `src/api/terminal_downlink.rs` | 695 | 下行编排与协议构造混合 |

结论：协议编解码已明显超出单文件可维护范围，后续应拆为目录模块（frame/codec/business/crypto/tests）。

## 2. 已修复的性能问题

### 2.1 车辆列表 PII N+1

原路径（`api/vehicles.rs`）：

- 每辆车 2 次 `get_pii` 查询
- 每辆车最多 2 次 `record_key_event` 插入
- 列表 50 辆：约 200 次数据库往返
- CSV 导出 1000 辆：约 4000 次数据库往返

修复后：

1. `get_pii_batch(owner_type, ids, fields)`：一次 `IN` 查询取回全部密文
2. `record_key_events(provider, action, successes)`：一个事务批量写审计
3. 新增 `hydrate_vehicles_pii(&mut [&mut Vehicle])`
4. `list` 与 `export` 改为先批量解密、再统一脱敏

效果：

| 场景 | 修复前 | 修复后 |
| --- | ---: | ---: |
| 50 辆列表 | ~200 次往返 | 1 次查询 + 1 个审计事务 |
| 1000 辆导出 | ~4000 次往返 | 1 次查询 + 1 个审计事务 |

### 2.2 PII 索引

`pii_vault` 原无索引，批量 `IN` 会全表扫描。新增迁移：

```sql
CREATE INDEX idx_pii_vault_owner
ON pii_vault (owner_type, owner_id, field)
```

MySQL 走不带 `IF NOT EXISTS` 的分支；PostgreSQL/SQLite 使用 `IF NOT EXISTS`。

### 2.3 角色感知 hydration

Viewer 本就被字段策略脱敏，不需要解密：

```rust
if can_view_key(role, "phone") || can_view_key(role, "driver_name") {
    hydrate_vehicles_pii(...)
}
```

实测（50 条列表）：

| 角色 | 优化前 | 优化后 |
| --- | ---: | ---: |
| Viewer | ~143 ms | ~6–9 ms |

### 2.4 审计批量与提交

- `record_key_events` 改为多行 INSERT（100 行/批）
- hydration 改用 `record_key_event_outcome`：每次批量最多 2 条审计（成功/失败各一）
- PostgreSQL 审计事务使用 `SET LOCAL synchronous_commit = off`

权衡：审计行在崩溃时可能丢失最近少量事件；业务数据写入仍使用服务器默认持久化。

### 2.5 密钥缓存

`vehicle_keyring()` 原每次调用都读取 `TSP_PII_KEY_FILE`。现按
`TSP_PII_KEY|TSP_PII_KEY_FILE` 环境标识缓存 `Arc<dyn KeyProvider>`，测试切换环境时自动重建。

### 2.6 验证

`tests/pii_vault.rs` 新增：

- 两辆车批量 hydration
- 两辆车手机号均正确解密
- 解密审计事件数量增加 4

## 3. 仍需处理的性能热点

| 位置 | 问题 | 建议 |
| --- | --- | --- |
| `Gb32960Signature::verify_rsa` | 每次验签解析 PEM 公钥 | 已增加 `verify_rsa_with_key`，调用方可缓存 |
| `Gb32960Signature::verify_sm2` | 每次构造 `VerifyingKey` 并算 ZA | 已增加 `verify_sm2_with_key`，调用方可缓存 |
| `Jt809Frame2019::encode_with_cipher` | body clone + inner copy + escape copy | 已改为 header+body 单缓冲、加密原地、转义直接写入输出 |
| `Gb32960Section::decode_all` | 每段 `Vec` 分配 | 预分配 + 借用切片视图 |
| `api/vehicles.rs` | 先解密全部字段再按角色脱敏 | 先按角色决定是否需要解密，减少无谓解密 |
| 全局 | 无 benchmark/负载基线 | 已加 `examples/bench_codecs.rs` 与 `scripts/bench_api.ps1` |
| API 写路径 | PG `fsync=on/open_datasync`，提交 200–800ms | 部署侧评估 NVMe/`commit_delay`；审计已放宽 |
| 测量环境 | Defender `MsMpEng` 可占满 CPU，API 数字会失真 | 压测前确认系统负载；记录 CPU/Defender 状态 |

## 4. 本机编解码基线

命令：

```text
cargo run --release --example bench_codecs
```

环境：本机 Windows / Rust release；非生产负载。

| 场景 | 吞吐 |
| --- | ---: |
| GB/T 32960 编帧（64B 数据） | ~2,539,000 ops/s |
| GB/T 32960 解帧 | ~1,441,000 ops/s |
| JT/T 809 编帧（256B body，全 0x5B 转义） | ~494,800 ops/s |
| JT/T 809 解帧 | ~395,200 ops/s |

说明：

- 809 body 全部为 `0x5B`，转义后帧长 548 字节，属于转义最坏情况。
- 32960 含 BCC 校验。
- 基准只覆盖编解码，不含 DB、加密、网络与媒体。

## 5. 结构建议

优先顺序：

1. `gb32960.rs` → `gb32960/{mod,frame,login,realtime,alarm,battery,crypto,monitor}.rs`
2. `jt809.rs` → `jt809/{mod,frame,header,crypto,login,platform,control,sequence}.rs`
3. `jt808/control.rs` → 按消息族拆分
4. 协议模块只保留编解码，禁止依赖 DB/HTTP
5. API 层只做授权、参数校验、编排

## 6. 测试基线

```text
lib                    164 passed
integration            31 passed
fmt                    clean
clippy                 clean
```

## 7. 结论

> 本轮消除了车辆列表/导出的 PII N+1，并补了索引；结构和密码缓存仍是下一阶段重点。  
> 当前没有真实设备/生产负载数据，不能给出吞吐或延迟承诺。
