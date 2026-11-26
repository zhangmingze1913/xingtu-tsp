# TSP 标准与生产缺口交接（2026-11-26）

## 1. 结论

```text
协议可模拟/可测试部分：已完成
生产环境：No-Go
第三方标准符合性：未认证
KGIS：仅可观测/PostGIS 兼容验证，无真实插件
```

## 2. 已完成

### JT/T 808

```text
注册 legacy-37 / 2019-table8 字段解析
鉴权、心跳、单点位置、0x0704 三点批量
重复单点/重复批量去重
TCP 分片重组
断线重连
```

测试：

```text
tests/standards_conformance.rs
tests/jt808_terminal_sim.rs
```

### JT/T 809

```text
mock 平台登录/报警/ACK/重连
TLS mock
会话状态机
```

测试：

```text
tests/jt809_sim.rs
tests/jt809_tls_sim.rs
```

### JT/T 1078

```text
信令
媒体包重排/重组
H.264 解析
乱序/重复/缺口跳过
修复 gap-skip 死循环
```

测试：

```text
tests/jt1078_sim.rs
```

### GB/T 32960

```text
登录/实时/补发/登出
报警/电池/电机/位置
签名固定向量
激活/应答
1h 高压监控状态机
```

测试：

```text
tests/gb32960_sim.rs
tests/standards_conformance.rs
```

### 平台/安全

```text
production 禁止 demo/模拟远控/admin123
production 强制 PII/KMS 与导出加密
生产 K8s overlay
PII 删除接口
health 输出 postgis/kgis/lat_lon_fallback
```

### 空间/KGIS 兼容

```text
真实 PostGIS 3.4
health/ready postgis 分支
bbox 查询命中
nearby 查询命中
```

测试：

```text
tests/postgis_health.rs（opt-in）
```

### 标准目录

```text
National/Industry/Local/Group
按 category/implementation 过滤
```

## 3. 测试基线

```text
lib          299 passed
integration  46 passed
fmt          clean
clippy       clean
```

命令：

```bash
cargo test --lib
cargo test --test auth --test vehicles --test standards --test storage \
  --test cluster --test standards_conformance --test jt808_terminal_sim \
  --test jt809_sim --test jt809_tls_sim --test gb32960_sim \
  --test jt1078_sim --test pii_vault --test standards_fuzz --test health \
  --test postgis_health
```

PostGIS 测试需要：

```powershell
$env:TSP_TEST_POSTGIS_URL='postgres://tsp:tsp-dev-password@127.0.0.1:55433/tsp?sslmode=disable'
```

## 4. 外部阻塞

| 项目 | 需要的输入 |
| --- | --- |
| JT/T 808 真实终端 | 终端/T-Box、抓包、弱网 |
| JT/T 809 平台互联 | 上下级平台地址、账号、证书 |
| JT/T 1078 视频 | 车载视频终端、流、弱网 |
| GB/T 32960 一致性 | 授权全文、官方测试集、检测机构 |
| KGIS | 插件包、License、Kingbase |
| 生产 HA | 多 AZ DB、托管 Redis、Ingress 证书 |
| 安全合规 | 渗透/等保/密码应用测评 |
| 灾备 | 备份/PITR/恢复演练 |
| 容量 | 生产规模压测/长稳 |
| 地方/团体 | 运营地监管接口/备案 |

## 5. 不能宣称

```text
不能宣称 JT/T 808/809/1078/GB/T 32960 已通过官方符合性
不能宣称 KGIS 已验证
不能宣称生产就绪
不能把模拟终端/平台/PostGIS 当成真实设备/真实 KGIS
```

## 6. 下一步

```text
1. 用真实终端替换 jt808_terminal_sim
2. 用真实视频终端跑 jt1078_sim 同等场景
3. 接真实 809 平台
4. 购买/授权官方标准与测试
5. 部署 KGIS
6. 多 AZ/灾备/安全测评
7. 生产容量长稳
```
