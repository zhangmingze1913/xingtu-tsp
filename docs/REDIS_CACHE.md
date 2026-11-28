# Redis session / read-through cache

## 1. 现有 Redis 状态

Redis 在集群模式中保存：

```text
tsp:session:<phone>       网关会话 owner/peer，TTL 90s
tsp:revoked:<jti>         JWT 吊销，TTL
tsp:node:<node_id>        节点存活
tsp:leader:scheduler      单例任务租约
tsp:gateway:<node_id>     Pub/Sub
tsp:jt808:kick            Pub/Sub
```

新增：

```text
tsp:terminal:<phone>      终端元数据/vehicle_id，TTL 60s
```

## 2. 终端 read-through

JT808 位置/批量高频路径：

```text
terminal_for(phone)
  -> Redis GET tsp:terminal:<phone>
  -> hit: 直接返回
  -> miss: PostgreSQL get_terminal_by_phone
          -> Redis SETEX 60s
```

认证仍走数据库 `verify_terminal_auth`，避免缓存 auth_code。

## 3. 失效点

```text
JT808 0x0100 注册       -> invalidate
API 终端绑定/解绑       -> invalidate
API 创建并绑定终端      -> invalidate
```

TTL 60s 是兜底；不能只依赖 TTL。

## 4. 指标

```text
xingtu_terminal_cache_hits_total
xingtu_terminal_cache_misses_total
```

目标：

```text
位置类请求缓存命中率 > 90%
失效后最多一次旧读
```

## 5. 失败模式

Redis 不可用：

```text
cached_terminal -> None
继续查 PostgreSQL
不阻塞网关
```

这是 fail-open，不是强一致缓存。

## 6. 百万连接注意事项

```text
1M 终端 key 需要单独 Redis 内存预算
建议 Redis Cluster，按 phone hash 分片
session key 与 terminal cache key 分 DB/实例
TTL 不宜过长
绑定变更必须失效
```

粗算：

```text
1M × (key + 200～500B value + overhead)
≈ 0.5–1.5 GiB
```

实际按 Redis `INFO memory` 压测确认。

## 7. 未完成

```text
Redis Cluster 百万 key 压测
缓存击穿 single-flight
多节点失效广播
终端缓存命中率/延迟 SLO
```

当前缓存是性能优化，不是业务真相源；PostgreSQL 仍是权威。
