# 性能剖析与火焰图

## 1. 当前工具状态

已在开发机检查：

```text
cargo flamegraph    未安装
perf                不可用（Windows）
samply 0.13.1       已安装
WPT/xperf           已安装（ADK ISO 提取 MSI）
```

本机：

```text
C:\Program Files (x86)\Windows Kits\10\Windows Performance Toolkit\xperf.exe
```

`samply record` 在 Windows 需要 **Windows Performance Toolkit (xperf)**：

```text
Could not start xperf: Could not find an xperf installation.
```

当前机器未安装 WPT，因此本轮**没有伪造火焰图**，使用 release 请求延迟/吞吐和 SQL 查询数作为证据。

## 2. Windows 火焰图

安装 WPT（只勾选 Windows Performance Toolkit）：

```text
https://learn.microsoft.com/windows-hardware/test/wpt/
```

安装后：

```powershell
samply record --duration 30 --save-only --no-open `
  -o profile.json.gz `
  -- .\target\release\xingtu-tsp.exe
```

采样期间压测：

```powershell
.\scripts\bench_api.ps1 -BaseUrl http://127.0.0.1:8090 `
  -Sequential 200 -Concurrency 20 -PerWorker 20
```

打开：

```powershell
samply open profile.json.gz
```

关注：

```text
tokio runtime
sqlx / AnyPool
serde_json
axum/tower
dashboard_overview
```

## 3. Linux/WSL 火焰图

```bash
cargo install flamegraph
cargo flamegraph --release --bin xingtu-tsp
```

需要 `perf_event_paranoid` 权限。容器内运行时需要 `--privileged` 或 `CAP_PERFMON`。

## 4. 建议采样场景

| 场景 | 负载 |
| --- | --- |
| 车辆列表 | 50 条 × 20 并发 |
| dashboard | 冷缓存 burst + 热缓存 |
| 轨迹 | 500/5000 点 |
| 遥测入库 | 单点 + 0x0704 批量 |
| 媒体 | 1078 包重组 |

## 5. 当前结构优化

即使没有火焰图，已确认并消除了两类结构性热点：

```text
dashboard 趋势：14 次 SQL -> 2 次
车辆汇总 + 状态分布：3 次扫描 -> 2 次
dashboard 冷缓存：per-org single-flight
```

## 6. 注意

- 火焰图是 CPU 视角，数据库等待/网络等待可能显示为 `epoll_wait`/`park`。
- 需要同时看 SQL 数量、连接池等待、锁竞争。
- 18 车/单机 KES 结果不能外推生产容量。
- 采样前必须预热，分别采冷/热两条路径。


## 7. 本机实际采样记录（2026-11-27）

WPT MSI 从 ADK ISO 提取安装，`xperf.exe` 存在；`samply record` 已成功生成：

```text
D:\tsp-profile-sqlite.json.gz
D:\tsp-profile-debug.json.gz
D:\tsp-profile-sym2.json.gz
```

采样负载：

```text
SQLite release：10x20 并发 824.9 req/s，0 失败
SQLite debug ：10x20 并发 225.3 req/s，0 失败
```

限制：

- 现有 release 无 PDB，栈是地址。
- debug 重建 `debuginfo` 后，samply 仍未能把 Rust 地址解析为函数名。
- 因此只保留 profile 文件与采样负载，不把地址栈当作有效火焰图结论。

可复现命令：

```powershell
samply record --duration 30 --save-only --no-open `
  --unstable-presymbolicate `
  -o D:\tsp-profile.json.gz `
  -- .\target\debug\xingtu-tsp.exe
```
