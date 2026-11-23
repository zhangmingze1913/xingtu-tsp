# Rust 代码结构（最近一次优化）

本轮继续在行为、SQL、测试不变的前提下，拆分纯适配层与大文件职责。

## 1. 当前体量

| 文件 | 行数 | 职责 |
| --- | ---: | --- |
| `db/mod.rs` | 239 | `Db`、连接池、方言、查询入口 |
| `db/seed_lock.rs` | 87 | 跨副本 seed 锁（PG advisory / MySQL GET_LOCK） |
| `db/map.rs` | 46 | 地图投影查询 |
| `db/tests.rs` | 129 | SQL 占位符/URL/方言单元测试 |
| `db/vehicles.rs` | 309 | 车辆 CRUD/分页 |
| `db/sql/mod.rs` | 10 | SQL 适配统一导出 |
| `db/sql/query.rs` | 180 | `DbQuery`/执行目标/编译缓存 |
| `db/sql/placeholders.rs` | 140 | `?N` 重写/MySQL 展开/DDL 拆分 |
| `db/sql/urls.rs` | 20 | SQLite/MariaDB URL 规范化 |
| `models/mod.rs` | 363 | 领域 DTO 主体 |
| `models/dashboard.rs` | 77 | 看板/地图 read model |
| `jt808/codec.rs` | 399 | 帧、BCD、转义、位置/注册解析 |
| `jt808/control.rs` | 100 | 0x8103/8105/8106/8202/8500 下行 body |
| `jt808/gateway.rs` | 309 | TCP 循环/分包/路由 |
| `jt808/gateway/dispatch.rs` | 139 | 消息分发 |
| `seed/demo.rs` | 299 | seed 编排 |
| `seed/fleet.rs` | 234 | 车辆/行程 builder |
| `seed/builders.rs` | 93 | 围栏/告警/指令 builder |

## 2. 模块边界

```text
src/
├── db/
│   ├── mod.rs              # Db + 连接池 + 方言入口
│   ├── seed_lock.rs        # seed 互斥
│   ├── map.rs              # 地图查询
│   ├── vehicles.rs         # 车辆 CRUD
│   ├── spatial.rs          # 附近/bbox
│   ├── dashboard.rs        # 看板
│   ├── telemetry*.rs       # 入库/规则/行程/查询
│   ├── sql/
│   │   ├── mod.rs          # re-export
│   │   ├── query.rs        # DbQuery
│   │   ├── placeholders.rs # ?N 适配
│   │   └── urls.rs         # URL 规范化
│   ├── migrations.rs       # 迁移
│   ├── postgis.rs          # 扩展/几何列/索引
│   ├── indexes.rs          # PostgreSQL 覆盖索引
│   └── tests.rs            # 纯 SQL 单元测试
├── models/
│   ├── mod.rs              # 领域 DTO
│   └── dashboard.rs        # 看板/地图 DTO
├── jt808/
│   ├── codec.rs            # 纯帧编解码
│   ├── control.rs          # 下行控制/参数 body
│   ├── gateway.rs          # 连接循环
│   └── gateway/            # dispatch/registration/location/downlink
└── seed/                   # demo/fleet/builders/model
```

## 3. 结构约束

- `db/mod.rs` 只保留 `Db`、连接池、方言判断和查询入口。
- `SeedLock` 独立模块，PG/MySQL/SQLite 语义集中在一处。
- `db/sql` 按职责拆为 URL、占位符、查询执行三部分，`DbQuery` 不再和 URL/SQL 拆分逻辑混排。
- `models/dashboard.rs` 只放读模型，领域实体与看板 DTO 分离。
- `jt808/control.rs` 只放下行 body；`codec.rs` 不依赖数据库、网络、会话。
- `gateway.rs` 不直接写 SQL；入库走 `location.rs`/`services`。
- 测试按域放置：SQL 适配在 `db/tests.rs`，API 行为在 `tests/*.rs`。
- `#![forbid(unsafe_code)]`、`pub(crate)` 默认、按需 `pub use`。

## 4. 验证

```text
cargo test --lib --test auth --test vehicles --test standards --test storage --test cluster
199 lib + 32 integration passed

cargo fmt --check        通过
cargo clippy --all-targets  无警告
```

## 5. 后续可继续

1. `models/mod.rs` 363 行：按 auth/vehicle/telemetry 继续拆。
2. `cluster.rs` 435 行：拆 session/leader/event 传输。
3. `jt808/codec.rs` 399 行：帧解析与 location/registration 解析分离。
4. `raft.rs`/`iggy_bus.rs`：按 transport/state machine 分层。
