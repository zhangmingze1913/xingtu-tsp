# Monoio runtime（opt-in）

## 1. 结论

Monoio 已作为 **可选 runtime/示例** 接入：

```toml
[features]
monoio-runtime = ["dep:monoio"]

[dependencies]
monoio = { version = "0.2", optional = true, features = ["macros"] }
```

示例：

```text
examples/monoio_gateway.rs
```

它不是当前 Axum/SQLx/Tokio 生产网关的替代品。

## 2. 为什么不做全量替换

当前主链路依赖：

```text
Axum HTTP
SQLx 数据库
Tokio runtime / mpsc / TLS / blocking pool
Redis / OpenRaft
```

Monoio 是 thread-per-core runtime，I/O trait、任务 spawn、TLS/DB 生态与 Tokio 不同。全量迁移会重写网关和 API，风险高于收益。

因此采用：

```text
保留 Tokio 生产路径
新增 monoio opt-in framing probe
后续只对 JT808 纯 TCP 网关单独评估
```

## 3. 示例能力

`monoio_gateway`：

```text
monoio TcpListener 127.0.0.1:18081
逐连接 read buffer
JT808 0x7e framing
parse_frame
0x8001 通用应答
```

不包含：

```text
数据库鉴权
Redis session
TLS
Axum API
```

所以它只能证明 monoio + JT808 codec 可运行，不是完整网关。

## 4. 平台

Monoio 官方支持：

```text
Linux（io_uring/epoll）
macOS（kqueue）
Windows：实验性/不完整
```

Windows 上 `cargo check --features monoio-runtime --example monoio_gateway` 可编译，但示例主逻辑是 Linux-only。

## 5. 构建

Linux / Kiri WSL：

```bash
cargo run --example monoio_gateway --features monoio-runtime
```

连接：

```bash
nc 127.0.0.1 18081
```

Windows 默认构建不会编译 monoio；不要在生产 Windows 上运行该 runtime。

## 6. 与 Kiri 的关系

Kiri 负责容器运行时，monoio 负责应用 runtime；两者独立。

Kiri 测试：

- [KIRI.md](KIRI.md)

Monoio 若要容器化，需要：

```text
Linux 容器
Rust 1.100 构建
--features monoio-runtime
io_uring/memlock 配置
```

本轮没有 monoio 容器压测。

## 7. 限制

```text
未做 monoio 全链路
未做 monoio TLS/DB
未做 io_uring 调优
未做 monoio 百万连接
```

结论：

> Monoio 已以 opt-in feature/example 接入，生产主路径仍是 Tokio。不能宣称 xingtu-tsp 已迁移到 monoio。
