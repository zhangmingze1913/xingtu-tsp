# PostgreSQL 16 主从复制验证

## 1. 结论

已用官方 `postgres:16` 容器验证：

```text
primary 16.16
replica 16.16
pg_basebackup -R
streaming / async
lag = 0 bytes
replica read = 主库数据
replica write = read-only 拒绝
手工 promote = 成功
```

这是**手工故障切换基线**，不是自动 failover 方案；生产仍需 Patroni/pg_auto_failover/云 RDS 等。

## 2. 拓扑

```text
pg-ha-primary  127.0.0.1:55434
pg-ha-replica  127.0.0.1:55435
network        tsp-ha
volume         pg-ha-replica
```

主库参数：

```text
wal_level=replica
max_wal_senders=10
hot_standby=on
```

复制账号：

```sql
CREATE ROLE repl WITH REPLICATION LOGIN PASSWORD 'replpass';
SELECT pg_create_physical_replication_slot('replica1');
```

`pg_hba.conf`：

```text
host replication repl 0.0.0.0/0 scram-sha-256
```

## 3. 复现

```powershell
pwsh -File scripts/postgres-ha-smoke.ps1 -Promote
```

脚本执行：

1. 创建 primary/replica 容器与复制卷
2. 创建复制账号和 slot
3. `pg_basebackup -Fp -Xs -R`
4. 启动 replica
5. 主库写 `ha_probe`
6. 从库读取
7. 从库写入必须失败
8. 可选 `-Promote`：停主库、提升从库、验证可写
9. 默认清理

保留环境：

```powershell
pwsh -File scripts/postgres-ha-smoke.ps1 -Keep
```

## 4. 实测输出

```text
pg_is_in_recovery = t
pg_stat_replication = streaming|async
lag_bytes = 0
replica SELECT = primary
replica INSERT = cannot execute INSERT in a read-only transaction
promote = server promoted
pg_is_in_recovery = f
promoted INSERT = ok
```

## 5. 应用接入建议

- 应用写连接指向 primary。
- 只读查询/报表可指向 replica，但要接受异步复制延迟。
- 不要把 replica 当强一致读。
- 应用启动迁移只应连 primary；连 replica 会因只读失败。
- 自动 failover 需要额外组件；本脚本只验证流复制与 promote 原语。

## 6. 生产检查

```text
[ ] 复制槽监控
[ ] WAL 保留/磁盘告警
[ ] 复制延迟告警
[ ] 自动 failover 组件
[ ] 客户端 VIP/DNS 切换
[ ] 脑裂防护
[ ] 备份与 PITR
```

未完成项不得用本 smoke 结果替代。


## 7. Patroni 自动 failover

手工 promote 之外，已用 etcd + 3 x Patroni/PostgreSQL 验证自动 failover。

镜像：

```text
ongres/patroni:latest   (Patroni 4.0.4, PostgreSQL 17.2)
quay.io/coreos/etcd:v3.5.17
```

拓扑：

```text
etcd
pg1 :55441 -> rest :8008
pg2 :55442 -> rest :8009
pg3 :55443 -> rest :8010
```

关键配置：

```yaml
scope: tsp
restapi:
  listen: 0.0.0.0:8008
  connect_address: pg1:8008
etcd3:
  hosts: etcd:2379
bootstrap:
  dcs:
    ttl: 30
    loop_wait: 10
    retry_timeout: 10
    postgresql:
      use_pg_rewind: true
      parameters:
        wal_level: replica
        hot_standby: "on"
        max_wal_senders: 10
  initdb:
    - encoding: UTF8
    - data-checksums
  pg_hba:
    - host all all 0.0.0.0/0 md5
    - host replication replicator 0.0.0.0/0 md5
postgresql:
  listen: 0.0.0.0:5432
  connect_address: pg1:5432
  data_dir: /data/pgdata
  bin_dir: /usr/lib/postgresql/17.2/bin
  authentication:
    superuser: {username: postgres, password: postgres}
    replication: {username: replicator, password: replicator}
  parameters:
    unix_socket_directories: /tmp
```

实测：

```text
pg1 Leader / pg2 Replica streaming / pg3 Replica streaming
stop pg1
pg3 Leader (TL 2)
INSERT failover_probe -> 1 row
pg2 pg_is_in_recovery = t
```

注意：

- 这是开发机单 etcd smoke；生产 etcd 必须 3/5 节点。
- 需要客户端 VIP/DNS/连接串切换。
- 需要 RPO/RTO、多 AZ、网络分区、脑裂演练。
- 应用迁移只连 leader。
