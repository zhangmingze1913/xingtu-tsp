# 安全加固说明

## 1. JWT：RS256 + JWKS + refresh/吊销

### 密钥

生产必须提供持久化 RSA 密钥，所有 API 节点共用同一对：

```bash
openssl genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:2048 -out jwt-private.pem
openssl rsa -pubout -in jwt-private.pem -out jwt-public.pem
```

```bash
TSP_JWT_PRIVATE_KEY_FILE=/etc/tsp/secret/jwt-private.pem
TSP_JWT_PUBLIC_KEY_FILE=/etc/tsp/secret/jwt-public.pem
TSP_JWT_KEY_ID=tsp-rs256-1
```

未配置时，本地/演示模式生成进程内临时密钥：
- 重启后旧 token 全部失效；
- 多节点之间 token 不互通；
- 不能作为生产配置。

### Token 生命周期

| Token | 默认 TTL | 存储 | 说明 |
| --- | ---: | --- | --- |
| access token | 15 分钟 | 客户端，无状态 | RS256，含 `jti/iss/aud/kid` |
| refresh token | 168 小时 | 客户端；服务端仅存 SHA-256 | 不透明随机串，一次一用 |

环境：

```bash
TSP_JWT_ACCESS_TTL_MINUTES=15
TSP_JWT_REFRESH_TTL_HOURS=168
TSP_JWT_ISSUER=xingtu-tsp
TSP_JWT_AUDIENCE=xingtu-api
```

### 接口

```http
POST /api/auth/login
POST /api/auth/refresh
POST /api/auth/logout
GET  /.well-known/jwks.json
```

登录响应保留旧字段 `token` 以兼容前端，同时返回：

```json
{
  "token": "<access>",
  "access_token": "<access>",
  "refresh_token": "<opaque>",
  "token_type": "Bearer",
  "expires_in": 900,
  "user": {}
}
```

刷新时旧 refresh token 立即吊销（轮换）；登出按 token hash 吊销 refresh token。
access token 登出时按 `jti` 写入 Redis 吊销键（单机降级为进程内缓存），后续请求在 `AuthUser` 提取器校验；多节点实测登出后其他节点立即 401。

### 多 key 轮换

旧公钥保留窗口：

```bash
TSP_JWT_PREVIOUS_KEYS='old-1=/etc/tsp/secret/jwt-old-1.pem'
```

- 新 token 只用当前私钥/`kid` 签发；
- 旧 `kid` 公钥继续验签并从 JWKS 发布；
- 保留时间应 ≥ access token TTL；
- 窗口结束后删除旧公钥，旧 token 自动失效；
- JWKS 会同时返回 active + previous keys。

### 数据库

迁移：

- `0010_refresh_tokens`
- `0011_refresh_tokens_user_index`

只保存 SHA-256 hash，不保存明文 refresh token。

## 2. Redis TLS / AUTH

客户端启用 `tokio-native-tls-comp`，支持：

```bash
TSP_REDIS_URL='rediss://:PASSWORD@redis.example.com:6379/0'
```

生产校验：

```bash
TSP_ENV=production
TSP_REDIS_ALLOW_PLAINTEXT=false
```

- `redis://` 且未显式允许 → 启动失败；
- `rediss://` 由系统原生 TLS 根证书校验；
- AUTH 通过 URL 中的 `:PASSWORD@` 传递；
- Redis ACL 用户可写成 `rediss://user:password@host:6379/0`。

真实 `rediss://` 联调：

- 本地 Redis 5 不直接支持 TLS，使用 TLS 终结代理 `127.0.0.1:56380 -> 56379`；
- 测试 CA 的 server 证书带 SAN `localhost`，临时导入当前用户 Root 信任；
- 应用使用 `rediss://:test-redis-pass@localhost:56380/8` 启动，`cluster=true`、健康检查通过；
- 测试后已撤销 Redis `requirepass` 并移除测试 CA。

生产私有 CA 需要把 CA 安装到容器系统信任链。

## 3. JT808 TLS / 专网

生产建议：

```bash
TSP_JT808_TLS_CERT=/etc/tsp/secret/jt808-cert.pem
TSP_JT808_TLS_KEY=/etc/tsp/secret/jt808-key.pem
TSP_JT808_ALLOW_PLAINTEXT=false
```

- 有证书：accept 后先做 TLS 1.3 握手，再进入 JT808 分帧；
- 无证书且 `ALLOW_PLAINTEXT=false`：网关启动失败；
- 无证书且显式允许：记录 warning，仅限专网/迁移期；
- 可选终端 mTLS：

```bash
TSP_JT808_CLIENT_CA=/etc/tsp/secret/jt808-client-ca.pem
```

- 可选 CIDR 白名单：

```bash
TSP_JT808_ALLOWED_CIDRS='10.0.0.0/8,192.168.0.0/16'
```

- 网关仍建议部署在私有网络，L4 LB 不直接暴露公网。

本机验证：

```text
JT808 TLS enabled cert=server.pem mtls=true
mTLS TLSv1.3 TLS_AES_256_GCM_SHA384
无客户端证书：peer sent no certificates（拒绝）
```

本地验证：

```text
TLS enabled cert=.../jt808-cert.pem
JT/T 808 gateway listening tls=true plaintext=false
TLSv1.3 TLS_AES_256_GCM_SHA384
```

## 4. 启动校验

`TSP_ENV=production` 会拒绝：

- 无 RS256 私钥；
- `redis://` 且 `TSP_REDIS_ALLOW_PLAINTEXT=false`；
- 无 JT808 证书且 `TSP_JT808_ALLOW_PLAINTEXT=false`；
- 默认 admin 密码、demo 模式、模拟指令；
- 默认开发 JWT 配置。

## 5. K8s

- `deploy/k8s/secret.example.yaml` 增加 RSA/JT808 PEM 占位；
- `api.yaml`/`gateway.yaml` 将 `tsp-secret` 挂载到 `/etc/tsp/secret`；
- `configmap.yaml` 配置 key 路径、TTL、TLS 开关；
- 真实密钥由 Secret/外部 secret manager 注入，禁止提交仓库。

## 6. 本轮完成与未完成

已完成：

- JWKS 多 key/旧 key 保留窗口；
- jti 本地 15s 正缓存；
- 真实 `rediss://` + AUTH + CA 信任链联调；
- JT808 mTLS（客户端证书）+ CIDR 白名单。

未完成（需要外部 IdP/产品决策，未在代码中伪造）：

- OIDC Authorization Code/PKCE 登录；
- SAML 2.0 SP/IdP 元数据与 XML 签名验证；
- TOTP/WebAuthn MFA、恢复码、登录挑战流程。

原因：OIDC/SAML 依赖真实 IdP 元数据、回调域名、签名证书和前端登录流程；MFA 会改变登录协议与用户管理 UI。当前保留了本地账号 + RS256 的稳定基线，避免引入未验证的半成品认证绕过面。

## 7. 剩余风险

- 本地对“未吊销”jti 做 5 秒正缓存，集群下跨节点登出生效窗口 ≤5 秒；当前节点立即生效；
- 缓存命中时不再访问 Redis；缓存未命中才 `EXISTS`；
- Redis 不可用时吊销检查失败会拒绝请求（fail-closed）；
- JWKS 单 key，尚未实现多 key 轮换/JWKS 缓存；
- JT808 为自签证书示例；生产需 CA 签发、终端证书校验/白名单；
- Redis TLS 使用系统根证书，私有 CA 需将 CA 安装到容器信任链；
- 标准合规仍需外部测试/认证，代码只是协议子集。
