# TDengine REST capacity smoke/benchmark for Windows PowerShell 5.1+.
#
# This is a bounded smoke benchmark, not a production capacity certification.
# It writes synthetic samples per vehicle and measures insert/query wall time.
#
# Examples:
#   .\scripts\tdengine-bench.ps1
#   .\scripts\tdengine-bench.ps1 -Vehicles 100 -SamplesPerVehicle 600 -BatchSize 100
#   .\scripts\tdengine-bench.ps1 -Cleanup
param(
    [string]$Url = 'http://127.0.0.1:6041',
    [string]$User = 'root',
    [string]$Password = 'taosdata',
    [string]$Database = 'tsp_bench',
    [int]$Vehicles = 50,
    [int]$SamplesPerVehicle = 200,
    [int]$BatchSize = 100,
    [switch]$Cleanup
)

$ErrorActionPreference = 'Stop'
$base = $Url.TrimEnd('/')
$pair = "$($User):$($Password)"
$auth = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($pair))
$headers = @{ Authorization = "Basic $auth" }

function Fail([string]$Message) {
    Write-Host "FAIL: $Message" -ForegroundColor Red
    exit 1
}

function Invoke-Taos([string]$Sql) {
    $response = Invoke-RestMethod -Method Post -Uri "$base/rest/sql" `
        -Headers $headers -ContentType 'text/plain' -Body $Sql -TimeoutSec 120
    if ($null -ne $response.code -and [int]$response.code -ne 0) {
        Fail "TDengine error $($response.code): $($response.desc)"
    }
    return $response
}

if ($Vehicles -lt 1 -or $SamplesPerVehicle -lt 1 -or $BatchSize -lt 1) {
    Fail 'Vehicles/SamplesPerVehicle/BatchSize must be positive'
}
if ($BatchSize -gt $SamplesPerVehicle) { $BatchSize = $SamplesPerVehicle }

Write-Host "TDengine capacity smoke"
Write-Host "  url:      $base"
Write-Host "  database: $Database"
Write-Host "  vehicles: $Vehicles"
Write-Host "  samples:  $SamplesPerVehicle per vehicle"
Write-Host "  batch:    $BatchSize"

Invoke-Taos "CREATE DATABASE IF NOT EXISTS ``$Database``" | Out-Null
Invoke-Taos "CREATE STABLE IF NOT EXISTS ``$Database``.``telemetry`` (ts TIMESTAMP, lat DOUBLE, lon DOUBLE, speed DOUBLE, heading DOUBLE, ignition BOOL, soc DOUBLE, odometer DOUBLE) TAGS (vehicle_id NCHAR(64))" | Out-Null

$startMs = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
$written = 0
$rateStart = [DateTime]::UtcNow

for ($v = 0; $v -lt $Vehicles; $v++) {
    $vid = "bench-$v"
    $table = "v_$($vid.Replace('-', '_'))"
    for ($offset = 0; $offset -lt $SamplesPerVehicle; $offset += $BatchSize) {
        $values = New-Object System.Collections.Generic.List[string]
        $count = [Math]::Min($BatchSize, $SamplesPerVehicle - $offset)
        for ($i = 0; $i -lt $count; $i++) {
            $ts = $startMs + ($offset + $i) * 1000
            $values.Add("($ts, 31.2, 121.4, 50.0, 90.0, true, 80.0, 100.0)")
        }
        $sql = "INSERT INTO ``$Database``.``$table`` USING ``$Database``.``telemetry`` TAGS ('$vid') VALUES " + ($values -join ',')
        Invoke-Taos $sql | Out-Null
        $written += $count
    }
    if (($v + 1) % 100 -eq 0) {
        Write-Host "  inserted vehicles: $($v + 1)"
    }
}

$writeSec = ([DateTime]::UtcNow - $rateStart).TotalSeconds
if ($writeSec -le 0) { $writeSec = 0.001 }
Write-Host ("OK:   wrote {0} rows in {1:N2}s ({2:N0} rows/s)" -f $written, $writeSec, ($written / $writeSec)) -ForegroundColor Green

$queryStart = [DateTime]::UtcNow
$count = Invoke-Taos "SELECT COUNT(*) FROM ``$Database``.``telemetry``"
$querySec = ([DateTime]::UtcNow - $queryStart).TotalSeconds
$total = $count.data[0][0]
Write-Host ("OK:   count query {0} rows in {1:N3}s" -f $total, $querySec) -ForegroundColor Green

$queryStart = [DateTime]::UtcNow
Invoke-Taos "SELECT COUNT(*), MIN(ts), MAX(ts) FROM ``$Database``.``telemetry`` WHERE vehicle_id = 'bench-0'" | Out-Null
$rangeSec = ([DateTime]::UtcNow - $queryStart).TotalSeconds
Write-Host ("OK:   per-vehicle range query in {0:N3}s" -f $rangeSec) -ForegroundColor Green

if ($Cleanup) {
    Invoke-Taos "DROP DATABASE IF EXISTS ``$Database``" | Out-Null
    Write-Host "OK:   dropped $Database" -ForegroundColor Green
} else {
    Write-Host "NOTE: $Database retained; pass -Cleanup to drop it."
}
