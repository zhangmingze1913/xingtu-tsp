# Kubernetes 完整链路 Smoke（k3d）

本页记录一次真实 k3d 验证：PostgreSQL + RustFS + JWT/JT808 TLS Secret + API。

> 这是单节点 smoke，不是生产高可用。生产需云 K8s/托管 PG/Redis/RustFS 与多副本。

## 1. 前置

```text
Docker Desktop
k3d v5.8.3
kubectl
xingtu-tsp:k8s 镜像
```

```bash
k3d cluster create tsp --servers 1 --agents 0 \
  --k3s-arg '--disable=traefik@server:0' --wait
kubectl get nodes
```

## 2. 生成密钥（不要提交）

```bash
openssl genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:2048 -out jwt-private.pem
openssl rsa -pubout -in jwt-private.pem -out jwt-public.pem
openssl genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:2048 -out jt808-key.pem
openssl req -new -x509 -key jt808-key.pem -out jt808-cert.pem -days 365 -subj "/CN=tsp-jt808"
```

或用测试脚本生成自签名证书。真实生产使用 KMS/Vault/Secret Manager。

## 3. 创建 namespace 与 Secret

```bash
kubectl create namespace xingtu-tsp

kubectl -n xingtu-tsp create secret generic tsp-secret \
  --from-literal=database-url='postgres://tsp:tsp-dev-password@postgres:5432/tsp?sslmode=disable' \
  --from-literal=postgres-password='tsp-dev-password' \
  --from-literal=rustfs-access-key='tsp-rustfs-admin' \
  --from-literal=rustfs-secret-key='tsp-rustfs-secret' \
  --from-literal=iggy-username='iggy' \
  --from-literal=iggy-password='iggy' \
  --from-file=jwt-private.pem \
  --from-file=jwt-public.pem \
  --from-file=jt808-cert.pem \
  --from-file=jt808-key.pem
```

## 4. 导入镜像

```bash
k3d image import xingtu-tsp:k8s -c tsp
```

数据镜像可让 K8s 拉取；离线环境逐个导入：

```bash
k3d image import postgres:16 rustfs/rustfs:latest redis:7-alpine -c tsp
```

## 5. 部署

```bash
kubectl apply -f deploy/k8s/configmap.yaml \
  -f deploy/k8s/postgres.yaml \
  -f deploy/k8s/redis.yaml \
  -f deploy/k8s/rustfs.yaml \
  -f deploy/k8s/api.yaml
```

单节点 smoke 覆盖：

```bash
kubectl -n xingtu-tsp patch configmap tsp-config --type merge --patch-file patch.json
kubectl -n xingtu-tsp scale deploy/tsp-api --replicas=1
kubectl -n xingtu-tsp set image deploy/tsp-api api=xingtu-tsp:k8s
kubectl -n xingtu-tsp set env deploy/rustfs RUSTFS_UNSAFE_BYPASS_DISK_CHECK=true
```

`patch.json`：

```json
{"data":{"TSP_CLUSTER_MODE":"false","TSP_REDIS_ALLOW_PLAINTEXT":"true","TSP_POSTGIS_ENABLED":"false"}}
```

等待：

```bash
kubectl -n xingtu-tsp get pods -w
kubectl -n xingtu-tsp rollout status deploy/tsp-api
```

## 6. 验证

```bash
kubectl -n xingtu-tsp port-forward svc/tsp-api 18092:8090
```

```bash
curl http://127.0.0.1:18092/api/health
curl -X POST http://127.0.0.1:18092/api/auth/login \
  -H 'Content-Type: application/json' \
  -d '{"username":"admin","password":"admin123"}'
```

本次实测：

```text
postgres-0                  1/1 Running
redis                       1/1 Running
rustfs                      1/1 Running
tsp-api                     1/1 Running
database_engine=postgres
vehicles total=18
JT808 TLS enabled
RustFS media archive enabled
archive prefix media/12345678901/1/
删除 API Pod 后重建：vehicles 仍为 18
```

## 7. 限制

```text
[ ] 多副本 API/PDB/HPA
[ ] 托管 PostgreSQL / 自动 failover
[ ] Redis HA/TLS
[ ] RustFS 多盘/多 Pool
[ ] Ingress TLS/证书管理
[ ] NetworkPolicy 实测
[ ] 备份/PITR
```

## 8. kind 失败记录

同一环境 kind v0.33.0 反复失败：

```text
kubeadm wait-control-plane
client rate limiter Wait ... context deadline
```

因此改用 k3d 完成本次完整链路验证。kind 失败是环境/控制面初始化问题，不代表应用清单错误。
