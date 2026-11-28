# Kiri 容器测试

## 1. 安装与运行

Kiri 0.2.2 Windows 安装包：

```text
https://download.runkiri.com/Kiri-0.2.2-x86_64.msi
```

安装后：

```text
%LOCALAPPDATA%\Kiri\bin\docker.exe
Docker context: kiri
WSL2 distro: kiri
```

验证：

```powershell
$d = "$env:LOCALAPPDATA\Kiri\bin\docker.exe"
& $d context ls
& $d --context kiri info
```

本机结果：

```text
27.5.1 Kiri
WSL2 Linux 6.18.58.1
/var/lib/kiri-daemon/.kiri/run/docker.sock
```

## 2. 镜像

导入已有镜像：

```powershell
& $d --context kiri load -i D:\xingtu-tsp-k8s.tar
```

本机：

```text
Loaded image: xingtu-tsp:k8s
```

## 3. 容器

```powershell
& $d --context kiri run -d --name tsp-kiri `
  -p 18100:8090 -p 18101:18080 `
  -e TSP_DATABASE=/data/tsp.db `
  -e TSP_DEMO_MODE=true `
  -e TSP_JWT_SECRET=kiri-secret `
  -e TSP_POSTGIS_ENABLED=false `
  -e TSP_TIMESERIES_BACKEND=postgres `
  -e TSP_CLUSTER_MODE=false `
  -e TSP_JT808_ALLOW_PLAINTEXT=true `
  xingtu-tsp:k8s
```

健康：

```text
/api/health database_engine=sqlite
```

## 4. 全链路

```powershell
.\scripts\fullchain-smoke.ps1 `
  -BaseUrl http://127.0.0.1:18100 `
  -Jt808Addr 127.0.0.1:18101
```

本机结果：

```text
HTTP 10x10：403.6 req/s，0 失败
JT808：鉴权 + 5 点全部 result=0
容器：60.09 MiB，CPU 0.00%（采样瞬间）
```

## 5. 限制

- 镜像是之前 `rust:1.94.1` 构建的；不是 1.100 镜像。
- Kiri 是容器运行时，不改变应用代码。
- Kiri Kubernetes 未在本轮启用；本轮是 Kiri Docker Engine。
- 百万连接未验证。
- 容器测试从此优先使用 Kiri context，而不是 Docker Desktop。

## 6. 常用命令

```powershell
$d = "$env:LOCALAPPDATA\Kiri\bin\docker.exe"
& $d --context kiri ps
& $d --context kiri logs -f tsp-kiri
& $d --context kiri stats tsp-kiri
& $d --context kiri rm -f tsp-kiri
```
