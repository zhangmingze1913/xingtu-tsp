# RustFS 对象存储

`xingtu-tsp` 使用 [RustFS](https://github.com/rustfs/rustfs) 作为 S3 兼容对象存储，替换原方案中的 MinIO。RustFS 采用 Apache-2.0 许可，提供 S3 API、Console、版本化、对象锁、生命周期和服务端加密等能力。

> 当前应用内的 1078 实时媒体仍保存在进程内 `MediaHub`，RustFS 是录制/归档媒体的部署目标。应用侧 S3 归档客户端尚未接入，详见文末“应用接入状态”。

## 端口与组件

| 组件 | 地址 | 用途 |
| --- | --- | --- |
| S3 API | `http://127.0.0.1:9000` | AWS CLI / SDK / 应用 |
| Console | `http://127.0.0.1:9001` | 管理界面 |
| 数据卷 | `/data/rustfs0..3` | 四个数据目录 |
| 日志卷 | `/app/logs` | RustFS 日志 |

本地默认凭据：

```text
RUSTFS_ACCESS_KEY=rustfsadmin
RUSTFS_SECRET_KEY=rustfsadmin
```

非本地环境必须修改，且不要把真实凭据提交到 Git。

## Docker Compose

```bash
docker compose up -d rustfs
```

启动后：

```bash
# 健康检查
curl -fsS http://127.0.0.1:9000/health
curl -fsS http://127.0.0.1:9001/rustfs/console/health

# 创建媒体桶（AWS CLI）
aws --endpoint-url http://127.0.0.1:9000 \
  s3 mb s3://tsp-media

# 上传/下载验证
aws --endpoint-url http://127.0.0.1:9000 \
  s3 cp ./sample.mp4 s3://tsp-media/sample.mp4
```

RustFS 容器以 UID/GID `10001:10001` 运行。Compose 中的 `rustfs-permissions` 一次性服务负责修正命名卷权限；绑定宿主机目录时需自行执行：

```bash
sudo chown -R 10001:10001 /path/to/data /path/to/logs
```

## Kubernetes

`deploy/k8s/rustfs.yaml` 提供单节点、单 PVC 的开发/预发部署，已加入 `kustomization.yaml`：

```bash
kubectl -n xingtu-tsp apply -f deploy/k8s/rustfs.yaml
```

凭据来自 `tsp-secret`：

```yaml
stringData:
  rustfs-access-key: "CHANGE_ME_RUSTFS_ACCESS_KEY"
  rustfs-secret-key: "CHANGE_ME_RUSTFS_SECRET_KEY"
```

生产环境应使用 RustFS Helm Chart 的多池、多盘、纠删码拓扑，而不是单 PVC 清单：

```text
https://charts.rustfs.com/
```

单盘拓扑不能原地扩容；扩容需要新建 Pool 或通过 S3 迁移数据。

## 从 MinIO 迁移

RustFS 与 MinIO 的 S3 API 兼容，但**默认不读取 MinIO 私有磁盘格式**。迁移应按对象级复制，而不是直接复用数据目录：

```bash
# 1. 从 MinIO 导出
aws --endpoint-url http://minio:9000 s3 sync s3://tsp-media ./tsp-media-export

# 2. 在 RustFS 建桶
aws --endpoint-url http://rustfs:9000 s3 mb s3://tsp-media

# 3. 导入 RustFS
aws --endpoint-url http://rustfs:9000 s3 sync ./tsp-media-export s3://tsp-media
```

应用侧只需替换：

```text
endpoint: http://minio:9000  ->  http://rustfs:9000
access_key / secret_key      ->  RustFS 凭据
bucket                       ->  同名桶或新桶
```

如果使用 S3 客户端签名，保持 `region` 与 `force_path_style` 配置一致即可。RustFS 默认不启用 MinIO 磁盘格式兼容；不要直接挂载 MinIO 数据目录。

## 安全建议

- 修改默认 `rustfsadmin/rustfsadmin`
- 生产启用 TLS，S3 端口不要直接暴露公网
- Console 仅内网或经反向代理 + SSO
- 为应用创建独立 access key，不使用管理员账号
- 开启版本化/对象锁用于取证与防篡改
- 配置生命周期，避免录制媒体无限增长

## 应用接入状态

当前状态：

- `docker-compose.yml`、`deploy/docker-compose.cluster.yml` 已包含 RustFS
- `deploy/k8s/rustfs.yaml` 已加入 Kustomization
- `.env.example` 已加入 RustFS 凭据
- 应用尚未使用 S3 SDK 上传录制媒体

下一步接入点：

```text
src/media/hub.rs       录制片段落盘/上传
src/api/media.rs       回放/下载时从 S3 读取
src/config.rs          TSP_S3_ENDPOINT / BUCKET / REGION / 凭据
```

在此之前，RustFS 可用于验证 S3 兼容性、桶策略和运维流程，但不要宣称“媒体已经写入 RustFS”。
