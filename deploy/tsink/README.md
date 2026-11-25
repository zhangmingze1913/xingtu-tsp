# tsink 本地容器

tsink 上游没有官方镜像/Dockerfile。以下 Dockerfile 用于 `cantrepro/tsink` 源码构建 `tsink-server`：

```bash
git clone https://github.com/cantrepro/tsink
cp deploy/tsink/Dockerfile tsink/
cd tsink
docker build -t tsp-tsink:dev .
docker run -d --name tsp-tsink -p 9201:9201 \
  -v tsp-tsink-data:/data tsp-tsink:dev \
  --listen 0.0.0.0:9201 --data-path /data
```

验证：

```bash
TSP_TEST_TSINK_URL=http://127.0.0.1:9201 \
  cargo test --lib tsink_container_roundtrip
```

注意：

- 不要传 `server` 子命令。
- Prometheus text label 必须带引号。
- 默认保留窗口会拒绝过旧时间戳。
