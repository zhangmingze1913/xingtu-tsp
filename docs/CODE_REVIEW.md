# Rust 代码组织结构审计

审计对象：`xingtu-tsp`（Rust 2021，Axum + Tokio + SQLx）。
本次审计按 Rust 项目惯例检查：模块边界、文件体量、可见性、错误处理、测试布局、依赖、格式化与 Clippy。

## 1. 结论摘要

| 项目 | 审计前 | 审计后 |
| --- | --- | --- |
| `src/db/mod.rs` | 约 1500 行 / 60 KB，DDL、行映射、SQL 适配、迁移、`Db` 混在一起 | 401 行；DDL→`schema/*.sql`，映射→`mappers.rs`，SQL 适配→`sql.rs`，迁移→`migrations.rs` |
| `src/jt808.rs` | 约 1310 行 / 48 KB，编解码、会话、下行、网关循环混在一起 | 231 行；codec 495 行、gateway 309 行（dispatch/registration/location/downlink 已拆） |
| `src/api/mod.rs` | 265 行，路由 + health + WebSocket 生命周期混合 | 18 行；router 121 行、health 20 行、WS 120 行 |
| `src/bootstrap.rs` | 217 行，DB/seed/cluster/可选 store 混在一起 | `bootstrap/mod.rs` 118 行 + cluster 47 + validate 63 |
| `src/seed/demo.rs` | 660 行，模型和全部 seed 逻辑混在一起 | demo 299 行 + fleet 234 行 + builders 93 行 + model 106 行 |
| `src/jt808/gateway.rs` | 596 行，注册/位置/下行/ACK 混在一起 | gateway 309 行 + dispatch 139 + registration 101 + location 78 + downlink 35 |
| `src/db/telemetry.rs` | 646 行 | 210 行 + `telemetry_rules.rs` 235 + `telemetry_trip.rs` 105 + `telemetry_queries.rs` 146 |
| 最大单文件 | `db/mod.rs` | `seed/demo.rs` 547 行，其余核心文件均 < 520 行 |
| crate 属性 | 无 crate 文档、无 unsafe 约束 | `#![forbid(unsafe_code)]` + crate 级文档 |
| 依赖 | `tempfile` 已不再使用 | 移除未使用 dev-dependency |
| 验证 | — | 36 lib + 8 集成测试通过，Clippy 无警告，fmt 通过 |

## 2. 模块结构（审计后）

```text
src/
├── main.rs                 # 进程入口：tracing + Config + run()
├── lib.rs                  # 应用装配、HTTP/worker 编排、crate 文档
├── bootstrap/              # mod/cluster/validate 启动装配
├── api/                    # HTTP 边界：路由、鉴权提取、handler
│   ├── mod.rs              # router/CORS/WS/错误转换
│   ├── vehicles.rs         # 车辆 CRUD
│   ├── telemetry.rs        # 遥测/轨迹 handler
│   ├── commands.rs         # 全局/车辆指令 handler
│   ├── terminal_downlink.rs# 808 下行 API
│   └── ...
├── db/
│   ├── mod.rs              # Db、连接池、占位符适配、迁移执行
│   ├── schema.rs           # include_str! DDL 入口
│   ├── schema/*.sql        # 三套可审阅 DDL
│   ├── mappers.rs          # AnyRow -> 领域模型 + 列投影
│   └── vehicles.rs ...     # 各聚合 repository
├── models.rs               # 跨层领域 DTO
├── jt808/
│   ├── mod.rs (jt808.rs)   # 网关会话、注册/鉴权/位置处理、下行
│   └── codec.rs            # 帧/BCD/位置/参数纯编解码
├── standards/              # JT/T 809、GB/T 32960 适配边界
├── cluster.rs              # Redis 会话/事件/租约
├── raft*.rs                # OpenRaft 网络、日志、状态机
├── services/               # 后台 worker / 指令分发
└── ...
```

分层原则：

- `api` 只做协议/鉴权/参数校验，不直接写 SQL。
- `db` 只做持久化；三套 DDL 与行映射独立文件。
- `jt808/codec` 无数据库/网络/会话状态，可被网关与模拟器复用。
- `standards` 只做标准帧/子集字段，不直接写库。
- `cluster`/`raft*` 只做协调，不承担业务数据。

## 3. 本次已修复的问题

### 3.1 `db/mod.rs` 职责过载

问题：DDL、SQL 方言适配、行解码、repository 全部在一个文件。

处理：

- `db/schema.rs`：三套 DDL 常量。
- `db/mappers.rs`：`AnyRow` 解码、列投影、行映射。
- `db/mod.rs`：保留 `Db`、连接池、`?N` 占位符适配、迁移。
- 通过 `pub(crate) use mappers::*` 保持现有 `super::{map_alert, ALERT_COLS}` 调用不变，避免大范围改动。

### 3.2 `jt808.rs` 协议与运行时耦合

问题：codec、会话注册表、下行状态、TCP 循环、业务入库在一个文件。

处理：

- `src/jt808/codec.rs`：消息常量、`Frame`、BCD/转义/校验、位置/注册解析、参数/车控 body、0x0201 响应。
- `jt808.rs`：保留会话注册表、pending 命令/下行状态、TCP 网关、`process_frame`、入库。
- `jt808.rs` 通过 `mod codec; pub use codec::*;` 保持公开 API 与模拟器 import 不变。

### 3.3 manifest 与 crate 约定

- 修复 `Cargo.toml` 的乱码 description。
- 移除未使用的 `tempfile` dev-dependency。
- `lib.rs` 增加 crate 文档和 `#![forbid(unsafe_code)]`。
- 检查生产代码无 `unwrap()/expect()`（仅测试/seed/bin 中使用）。

## 4. 本轮后续六项完成情况

| 项 | 结果 |
| --- | --- |
| `AppState` 构造器重复 | 引入 `AppStateBuilder`，`new/cluster/cluster_and_iggy/tsink/tdengine/walrus` 统一组装；字段改为 crate 可见 |
| `lib.rs::run` 偏长 | 抽出 `src/bootstrap.rs`：安全校验、DB、seed、cluster、Iggy、tsink、TDengine、Walrus 全部移入 |
| 三套 DDL | 移至 `src/db/schema/{sqlite,postgres,mysql}.sql`，`schema.rs` 用 `include_str!` |
| `api/vehicles.rs` | 遥测拆到 `api/telemetry.rs`，指令拆到 `api/commands.rs`，车辆 CRUD 留在原文件 |
| `api/mod.rs` | health/ws 拆出；`api/mod.rs` 只保留 router/CORS/静态资源/错误转换 |
| `db/mod.rs` | 迁移→`db/migrations.rs`，SQL 适配→`db/sql.rs`，DDL→`db/schema/*.sql`，`mod.rs` 401 行 |
| `seed.rs` | 拆为 `seed/admin.rs`、`seed/demo.rs`、`seed/mod.rs` |
| `db/telemetry.rs` | 查询/离线/保留拆到 `db/telemetry_queries.rs` |
| `jt808.rs` | 网关循环拆到 `jt808/gateway.rs`，父模块 231 行 |
| `bootstrap` | 拆为 `bootstrap/mod.rs`、`cluster.rs`、`validate.rs` |
| `seed` | 拆为 `seed/admin.rs`、`seed/demo.rs`、`seed/builders.rs`、`seed/model.rs`、`seed/mod.rs` |
| `jt808/gateway` | 拆为 `gateway.rs` + `registration.rs` / `location.rs` / `downlink.rs` |
| `telemetry` | 查询/离线/保留→`telemetry_queries.rs`，告警规则→`telemetry_rules.rs` |
| `tests/api.rs` | 拆为 `tests/common/mod.rs` + `auth.rs` / `vehicles.rs` / `standards.rs` / `storage.rs` / `cluster.rs` |
| `pub mod` 面过大 | `api/cluster/iggy_bus/metrics/raft*/realtime/services/tdengine/timeseries/util/walrus_bus` 改为私有；仅 re-export 模拟器需要的 808 codec 与 `parse_peers` |

## 5. 仍存在的结构问题（建议后续）

1. `seed/fleet.rs` 234 行：可继续拆车辆轨迹/行程 builder。
2. `jt808/gateway.rs` 309 行：可继续拆分包重组。
3. `db/telemetry.rs` 210 行：可继续拆定位入库。
4. `bootstrap/mod.rs` 118 行：可选 store 可再拆 `stores.rs`。
5. 标准协议业务消息体与 1078/19056 仍不在本仓库完整实现范围。

## 6. Rust 规范检查清单

| 检查项 | 结果 |
| --- | --- |
| `cargo fmt --check` | 通过 |
| `cargo clippy --all-targets` | 无警告 |
| `cargo test`（lib + 5 个集成测试文件） | 42 + 11 通过 |
| `unsafe` | `#![forbid(unsafe_code)]` |
| 生产 `unwrap/expect` | 未发现（测试/seed/bin 固有场景除外） |
| 错误传播 | 统一 `AppResult` / `thiserror`（协议层） |
| 模块文档 | 根 crate、db、jt808 codec、standards 均有模块文档 |
| 测试布局 | 单元测试与源码同文件；HTTP 集成测试按域拆分 |
| 依赖 | 移除未使用 `tempfile`；`openraft-legacy` 直接依赖已显式声明 |

## 7. 验收命令

```bash
cargo fmt --check
cargo clippy --all-targets
cargo test --lib --test auth --test vehicles --test standards --test storage --test cluster
```

本次拆分后已执行以上命令并通过。
