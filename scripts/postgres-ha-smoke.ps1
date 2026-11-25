# PostgreSQL 16 streaming-replication smoke test.
#
# Creates a primary and a physical replica with pg_basebackup, verifies
# streaming, read-only replica semantics, optional promotion, then cleans up.
#
# Usage:
#   pwsh -File scripts/postgres-ha-smoke.ps1
#   pwsh -File scripts/postgres-ha-smoke.ps1 -Promote
#   pwsh -File scripts/postgres-ha-smoke.ps1 -Keep
param(
    [string]$Network = "tsp-ha",
    [string]$Primary = "pg-ha-primary",
    [string]$Replica = "pg-ha-replica",
    [string]$Volume = "pg-ha-replica",
    [int]$PrimaryPort = 55434,
    [int]$ReplicaPort = 55435,
    [switch]$Promote,
    [switch]$Keep
)

$ErrorActionPreference = "Stop"

function Wait-Pg([string]$Container) {
    for ($i = 0; $i -lt 60; $i++) {
        docker exec $Container pg_isready -U tsp -d tsp *> $null
        if ($LASTEXITCODE -eq 0) { return }
        Start-Sleep -Seconds 1
    }
    throw "$Container did not become ready"
}

function Run-Psql([string]$Container, [string]$Sql) {
    docker exec $Container psql -U tsp -d tsp -v ON_ERROR_STOP=1 -c $Sql
    if ($LASTEXITCODE -ne 0) { throw "psql failed on $Container" }
}

if (-not (docker network ls --format '{{.Name}}' | Select-String -SimpleMatch $Network)) {
    docker network create $Network | Out-Null
}
foreach ($name in @($Primary, $Replica)) {
    if (docker ps -a --format '{{.Names}}' | Select-String -SimpleMatch $name) {
        docker rm -f $name | Out-Null
    }
}
if (docker volume ls --format '{{.Name}}' | Select-String -SimpleMatch $Volume) {
    docker volume rm $Volume | Out-Null
}
docker volume create $Volume | Out-Null

docker run -d --name $Primary --network $Network `
    -e POSTGRES_USER=tsp -e POSTGRES_PASSWORD=tsp-dev-password -e POSTGRES_DB=tsp `
    -p "${PrimaryPort}:5432" postgres:16 `
    -c wal_level=replica -c max_wal_senders=10 -c hot_standby=on | Out-Null
Wait-Pg $Primary

Run-Psql $Primary "CREATE ROLE repl WITH REPLICATION LOGIN PASSWORD 'replpass'"
Run-Psql $Primary "SELECT pg_create_physical_replication_slot('replica1')"
docker exec $Primary bash -c `
    "grep -q 'host replication repl' /var/lib/postgresql/data/pg_hba.conf || echo 'host replication repl 0.0.0.0/0 scram-sha-256' >> /var/lib/postgresql/data/pg_hba.conf"
Run-Psql $Primary "SELECT pg_reload_conf()"

docker run --rm --network $Network -v "${Volume}:/var/lib/postgresql/data" postgres:16 `
    bash -c "chown -R postgres:postgres /var/lib/postgresql/data && su postgres -c 'PGPASSWORD=replpass pg_basebackup -h $Primary -U repl -D /var/lib/postgresql/data -Fp -Xs -P -R -S replica1'"
if ($LASTEXITCODE -ne 0) { throw "pg_basebackup failed" }

docker run -d --name $Replica --network $Network `
    -v "${Volume}:/var/lib/postgresql/data" -p "${ReplicaPort}:5432" postgres:16 | Out-Null
Wait-Pg $Replica

$recovery = docker exec $Replica psql -U tsp -d tsp -Atc "SELECT pg_is_in_recovery()"
if ($recovery.Trim() -ne "t") { throw "replica is not in recovery" }

Run-Psql $Primary "CREATE TABLE IF NOT EXISTS ha_probe(id int primary key, note text); INSERT INTO ha_probe VALUES (1,'primary') ON CONFLICT DO NOTHING"
Start-Sleep -Seconds 1
$value = docker exec $Replica psql -U tsp -d tsp -Atc "SELECT note FROM ha_probe WHERE id=1"
if ($value.Trim() -ne "primary") { throw "replica did not receive primary row" }

$oldPreference = $ErrorActionPreference
$ErrorActionPreference = "Continue"
docker exec $Replica psql -U tsp -d tsp -c "INSERT INTO ha_probe VALUES (99,'must-fail')" *> $null
$replicaWriteExit = $LASTEXITCODE
$ErrorActionPreference = $oldPreference
if ($replicaWriteExit -eq 0) { throw "replica accepted a write" }

$replication = docker exec $Primary psql -U tsp -d tsp -Atc `
    "SELECT state || '|' || sync_state FROM pg_stat_replication"
Write-Output "replication=$($replication.Trim())"

if ($Promote) {
    docker stop $Primary | Out-Null
    docker exec $Replica su postgres -c "pg_ctl promote -D /var/lib/postgresql/data"
    Start-Sleep -Seconds 2
    $recovery = docker exec $Replica psql -U tsp -d tsp -Atc "SELECT pg_is_in_recovery()"
    if ($recovery.Trim() -ne "f") { throw "replica promotion failed" }
    Run-Psql $Replica "INSERT INTO ha_probe VALUES (2,'promoted')"
    Write-Output "promotion=ok rows=2"
}

if (-not $Keep) {
    foreach ($name in @($Primary, $Replica)) {
        if (docker ps -a --format '{{.Names}}' | Select-String -SimpleMatch $name) {
            docker rm -f $name | Out-Null
        }
    }
    if (docker volume ls --format '{{.Name}}' | Select-String -SimpleMatch $Volume) {
        docker volume rm $Volume | Out-Null
    }
    if (docker network ls --format '{{.Name}}' | Select-String -SimpleMatch $Network) {
        docker network rm $Network | Out-Null
    }
}

Write-Output "postgres-ha-smoke=ok"
