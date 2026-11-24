# RustFS 对象存储

`xingtu-tsp` 使用 [RustFS](https://github.com/rustfs/rustfs) 作为 S3 兼容对象存储，替换原方案中的 MinIO。RustFS 采用 Apache-2.0 许可，提供 S3 API、Console、版本化、对象锁、生命周期和服务端加密等能力。

> 1078 实时媒体继续由进程内 `MediaHub` 提供低延迟快照/连续流；同时按关键帧/时间分段，将 raw H.264/AAC 上传 RustFS 归档。归档对象可通过 API 列出和下载。

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

## 应用接入

### 配置

```bash
TSP_MEDIA_UDP_ADDR=0.0.0.0:19000
TSP_RUSTFS_ARCHIVE=true
TSP_RUSTFS_ENDPOINT=http://rustfs:9000
TSP_RUSTFS_BUCKET=tsp-media
TSP_RUSTFS_REGION=us-east-1
TSP_RUSTFS_ACCESS_KEY=rustfsadmin
TSP_RUSTFS_SECRET_KEY=rustfsadmin
TSP_RUSTFS_PREFIX=media
TSP_RUSTFS_SEGMENT_SECS=30
TSP_RUSTFS_SEGMENT_BYTES=8388608
TSP_RUSTFS_SEGMENT_MAX_BYTES=67108864
TSP_RUSTFS_UPLOAD_RETRIES=3
TSP_RUSTFS_UPLOAD_TIMEOUT_SECS=30
TSP_RUSTFS_UPLOAD_CONCURRENCY=4
TSP_RUSTFS_RETENTION_DAYS=0
TSP_RUSTFS_CLEANUP_INTERVAL_SECS=3600
TSP_ARCHIVE_PLAYBACK_CACHE_ITEMS=8
TSP_ARCHIVE_PLAYBACK_CACHE_BYTES=67108864
TSP_ARCHIVE_SPOOL_DIR=data/archive-spool
TSP_ARCHIVE_SPOOL_REPLAY_SECS=60
```

`TSP_RUSTFS_ARCHIVE` 未开启时，录制和上传完全关闭，`MediaHub` 行为与之前一致。

启动时应用会调用 HeadBucket；桶不存在时自动创建。生产建议预创建桶并收紧应用凭据权限。

### 录制行为

- 数据来源：`TSP_MEDIA_UDP_ADDR` 上的 1078 媒体包
- 视频：H.264 Annex-B，`video/h264`
- 音频：AAC ADTS，`audio/aac`
- 分段触发：
  - 视频段只从 I 帧开始，首个 I 帧前的视频帧丢弃
  - 遇到 I 帧且达到 `TSP_RUSTFS_SEGMENT_SECS` 或 `TSP_RUSTFS_SEGMENT_BYTES`
  - 没有 I 帧导致缓冲超过 `TSP_RUSTFS_SEGMENT_MAX_BYTES` 时丢弃并告警
  - 音频流达到时间/大小时独立切分
  - 会话 `DELETE` 时强制冲刷
- 会话 `clear` 会触发当前分段上传
- 上传异步执行，不阻塞 1078 接收循环
- 每次上传受并发信号量限制；失败按指数退避重试；单次尝试有超时

对象键：

```text
media/{sim}/{channel}/YYYY/MM/DD/HHMMSS-{sequence}.h264
media/{sim}/{channel}/YYYY/MM/DD/HHMMSS-{sequence}.aac
```

### 归档清理

- `TSP_RUSTFS_RETENTION_DAYS=0` 关闭清理
- 大于 0 时按 S3 `LastModified` 删除过期对象
- 启动后按 `TSP_RUSTFS_CLEANUP_INTERVAL_SECS` 周期执行
- 没有 `LastModified` 的对象不会被删除
- 只清理配置前缀下对象

### 失败上传暂存与重放

- 上传重试耗尽后，将 key/类型/内容写入 `TSP_ARCHIVE_SPOOL_DIR`
- 每个对象两个文件：`<sha256>.json` 元数据 + `<sha256>.bin` 内容
- 元数据最后写入，作为完成标记
- 后台每 `TSP_ARCHIVE_SPOOL_REPLAY_SECS` 秒扫描并重试
- 成功后删除暂存文件
- 暂存目录设为 `off` 可关闭
- 指标：`xingtu_archive_upload_spooled_total`、`xingtu_archive_spool_replayed_total
xingtu_archive_spool_pending`

注意：K8s 默认使用 `/tmp/archive-spool`，Pod 重建会丢失；生产应挂载持久卷或改用对象存储队列。

### 回放缓存

- `TSP_ARCHIVE_PLAYBACK_CACHE_ITEMS` 默认 8 条
- `TSP_ARCHIVE_PLAYBACK_CACHE_BYTES` 默认 64MB
- LRU，按条数与总字节双重淘汰
- 单条超过字节上限时跳过缓存
- FFmpeg remux 失败返回 503 并记录 warn 日志

### 归档 API

列出对象：

```http
GET /api/media/archive?sim=138000000001&channel=1
Authorization: Bearer <token>
```

下载对象：

```http
GET /api/media/archive/media/138000000001/1/2026/11/24/080001-000000.h264
Authorization: Bearer <token>
```

归档回放（服务端把同名 `.h264`/`.aac` remux 为 fMP4）：

```http
GET /api/media/archive/playback?key=media/138000000001/1/2026/11/24/080001-000000.h264
Authorization: Bearer <token>
```

`key` 必须带 `TSP_RUSTFS_PREFIX` 前缀，且拒绝 `..`、反斜杠和目录结尾。

### 播放器 UI

`/media` 页面新增“归档列表”：

- 输入/选择 SIM 与通道
- 调 `GET /api/media/archive` 列出对象
- 按视频/音频过滤
- 每页 20 条，上一页/下一页
- “播放”调用归档回放接口，经 MSE 播放
- “下载”带 Token 拉取原文件

### 真实 RustFS 联调脚本

```powershell
.\scripts\rustfs-smoke.ps1
```

脚本执行：

- S3 `/health` 与 Console `/health`
- 创建桶
- 上传 `assets/sample.h264`
- `s3 ls` 验证
- 下载并比对 SHA-256

依赖 AWS CLI v2；RustFS 需先启动。

### 指标

`/api/metrics` 新增：

```text
xingtu_archive_upload_success_total
xingtu_archive_upload_failure_total
xingtu_archive_upload_retries_total
xingtu_archive_segments_dropped_total
xingtu_archive_cleanup_deleted_total
xingtu_archive_playback_success_total
xingtu_archive_playback_failure_total
xingtu_archive_playback_cache_hit_total
xingtu_archive_playback_cache_miss_total
```

### 代码位置

```text
src/media/archive.rs   RustFS S3 客户端、对象键、上传/列表/下载
src/media/hub.rs       分段缓冲、I 帧/时间/大小触发、异步上传
src/api/media.rs       归档列表/下载 API
src/lib.rs             启动时确保桶存在
```

### 测试

- `media_keys_are_prefixed_and_path_safe`
- `prefix_and_segment_limits_have_safe_defaults`
- `archive_playback_pairs_video_and_audio_keys`
- `upload_retries_transient_s3_failures`
- `cleanup_deletes_only_expired_objects`
- `video_segments_start_at_keyframes`
- `cache_is_lru_by_item_count`
- `cache_evicts_by_bytes_and_skips_oversize`
- `spool_save_pending_and_remove_roundtrip`
- `spool_id_is_stable_and_path_safe`
- `replay_spool_uploads_and_removes_item`
- `render_includes_archive_counters`
- `archive_put_list_get_against_s3_mock`：真实 S3 客户端对 mock 发起 PUT/GET/ListObjectsV2

未执行：

- 真实 RustFS 容器的上传/下载（脚本已提供，本机无 Docker/AWS CLI 未运行）
- 断点、限流、大文件、并发上传
- 真实 1078 终端产生的长分段回放

### 限制

- 归档原对象为 raw Annex-B/ADTS；播放接口在服务端 remux 为 fMP4
- 分段以 I 帧为优先边界；兜底切分可能从非 I 帧开始
- 音频/视频分段分别上传，不保证同名对象原子成对
- 未实现归档生命周期、对象锁、跨区域复制
