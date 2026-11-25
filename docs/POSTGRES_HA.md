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
