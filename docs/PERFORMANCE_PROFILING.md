# 性能剖析与火焰图

## 1. 当前工具状态

已在开发机检查：

```text
cargo flamegraph    未安装
perf                不可用（Windows）
samply 0.13.1       已安装
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
