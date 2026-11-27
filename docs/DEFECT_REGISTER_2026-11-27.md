# TSP 缺陷与未实现功能登记（2026-11-27）

> 本表只记录可验证事实。`Open` 不等于“代码没写”，也可能是需要真实设备/授权/检测的 P0。

## 1. 本轮已修复

| ID | 缺陷 | 修复 | 回归 |
| --- | --- | --- | --- |
| F-01 | JT/T 1078 序列缺口跳过时只更新映射、不更新局部游标，导致死循环 | `expected = next`，并保留迭代上限 | `stream_pipeline_reorders_duplicates_and_skips_gaps` |
| F-02 | catalog 将 JT/T 809 经典帧标为 `Done`，但经典帧明确拒绝加密/压缩 | 改为 `Partial`，名称标注缺口 | `jt809_frame_is_not_fully_done` |
| F-03 | production 可静默使用明文 PII/明文导出 | 启动强制 PII/KMS、`TSP_EXPORT_REQUIRE_ENCRYPTION=true`、导出密钥 | `production_secret_policy_rejects_plaintext_paths` |
| F-04 | 无生产 K8s overlay，PII/导出密钥未接入 | 新增 `deploy/overlays/production`，`kubectl kustomize` 校验 | manifest 校验 |
| F-05 | health 无法区分空间能力 | 增加 `postgis/kgis/lat_lon_fallback` | `health_reports_spatial_backend` |
| F-06 | 删除车辆只删 `vehicles`，`pii_vault` 无外键级联，遗留孤儿 PII | 同事务删除 `pii_vault` + vehicle | `vehicle_delete_removes_vaulted_pii` |
| F-07 | 畸形输入 fuzz 仅覆盖 5 个入口/200 组 | 扩到 7 个入口/1000 组，覆盖 808 注册/位置/批量、809 2019 头、32960 签名/激活 | `malformed_frames_never_panic` |

## 2. 未完全实现（Open）

| ID | 功能 | 现状 | 验收条件 |
| --- | --- | --- | --- |
| O-01 | JT/T 809 经典帧加密/压缩 | `Jt809Frame::decode` 返回 `Encrypted`/`Compressed` | 授权全文 + 真实/模拟密文向量 |
| O-02 | JT/T 808 加密报文 | `ProtocolError::Encrypted` | 标准加密模式、密钥协商、终端联调 |
| O-03 | GB/T 32960 密文模式 | 有 `decode_with_decrypt` 回调，但模式/密钥由调用方决定 | 授权全文明确 SM4 模式 + 官方测试集 |
| O-04 | 真实 JT/T 808 终端 | 仅模拟终端 | 真机注册/鉴权/位置/批量/断网 |
| O-05 | 真实 JT/T 1078 视频 | 合成 H.264/AAC、乱序/缺口 | 真机实时/回放/弱网 |
| O-06 | 真实 JT/T 809 平台 | TCP/TLS mock | 上下级平台登录/报警/监管 |
| O-07 | KGIS | PostGIS 兼容 + health 标记 | KGIS 插件/License/Kingbase 空间索引 |
| O-08 | 官方标准符合性 | 本地向量/模拟 | GB/T 32960 官方测试集/检测机构 |
| O-09 | 生产 HA/灾备 | Patroni smoke、单节点 k3d | 多 AZ、RPO/RTO、PITR 演练 |
| O-10 | 生产安全测评 | 代码控制、生产 overlay | 渗透/等保/密码应用测评 |
| O-11 | 生产容量 | 18 车 KES、PostGIS 1M 专项 | 生产规模压测/长稳/SLO |
| O-12 | 地方/团体落地 | catalog + 分类过滤 | 运营地监管接口/备案/合同要求 |
| O-13 | PII 备份删除 | 在线删除 `pii_vault`/字段 | 备份/副本/导出保留策略与演练 |
| O-14 | 火焰图 | samply 可用、xperf/WPT 缺失 | 安装 WPT 或 Linux perf 采样 |

## 3. 风险说明

- O-01/O-02 是**协议能力缺口**，不能靠文档或 mock 关闭。
- O-04~O-07 是**真实设备/平台/数据库插件**缺口。
- O-08~O-11 是**第三方/生产环境**缺口。
- O-13 是**运维/合规**缺口，在线删除不等于备份删除。
- O-14 只影响性能剖析深度，不影响功能正确性。

## 4. 当前基线

```text
lib          300 passed
integration  46 passed
fmt          clean
clippy       clean
```

## 5. 结论

> 当前仍不能宣称生产就绪或标准符合性。已修复可复现缺陷；未实现项已按协议、设备、第三方检测、生产运维分类登记，并给出验收条件。
