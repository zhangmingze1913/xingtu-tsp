param(
    [string]$ApiBase = "http://127.0.0.1:8090",
    [int]$PgPort = 55432,
    [int]$RedisPort = 56379,
    [int]$MinFreeGb = 1
)

$ErrorActionPreference = "Stop"
$failures = New-Object System.Collections.Generic.List[string]

function Check-Name([string]$name, [bool]$ok, [string]$detail = "") {
    if ($ok) { Write-Output "PASS $name $detail" }
    else { Write-Output "FAIL $name $detail"; $script:failures.Add($name) | Out-Null }
}

$drive = Get-PSDrive C
$freeGb = [math]::Round($drive.Free / 1GB, 2)
Check-Name "disk>=${MinFreeGb}GB" ($freeGb -ge $MinFreeGb) "free=${freeGb}GB"

$pg = Test-NetConnection -ComputerName 127.0.0.1 -Port $PgPort -InformationLevel Quiet -WarningAction SilentlyContinue
Check-Name "postgres" ([bool]$pg) "port=$PgPort"
$redis = Test-NetConnection -ComputerName 127.0.0.1 -Port $RedisPort -InformationLevel Quiet -WarningAction SilentlyContinue
Check-Name "redis" ([bool]$redis) "port=$RedisPort"

try {
    $health = Invoke-RestMethod -Uri "$ApiBase/api/health" -TimeoutSec 5
    Check-Name "api-health" ($health.status -eq "ok") "db=$($health.database)"
    $ready = Invoke-RestMethod -Uri "$ApiBase/api/ready" -TimeoutSec 5
    Check-Name "api-ready" ([bool]$ready.ready) "leader=$($ready.cluster_leader)"
} catch {
    Check-Name "api" $false $_.Exception.Message
}

if ($failures.Count -gt 0) {
    Write-Output "RESULT=FAIL count=$($failures.Count)"
    exit 1
}
Write-Output "RESULT=PASS"
