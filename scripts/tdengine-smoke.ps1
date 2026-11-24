# TDengine REST smoke test for Windows PowerShell 5.1+.
#
# Requires a running TDengine 3.x taosAdapter (default :6041).
#
# Examples:
#   .\scripts\tdengine-smoke.ps1
#   .\scripts\tdengine-smoke.ps1 -Url http://127.0.0.1:6041 -Database tsp
param(
    [string]$Url = 'http://127.0.0.1:6041',
    [string]$User = 'root',
    [string]$Password = 'taosdata',
    [string]$Database = 'tsp',
    [string]$VehicleId = 'smoke-vehicle'
)

$ErrorActionPreference = 'Stop'
$base = $Url.TrimEnd('/')

function Fail([string]$Message) {
    Write-Host "FAIL: $Message" -ForegroundColor Red
    exit 1
}

function Ok([string]$Message) {
    Write-Host "OK:   $Message" -ForegroundColor Green
}

function Invoke-TaosSql([string]$Sql) {
    $pair = "$($User):$($Password)"
    $auth = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($pair))
    try {
        $response = Invoke-RestMethod -Method Post -Uri "$base/rest/sql" `
            -Headers @{ Authorization = "Basic $auth" } `
            -ContentType 'text/plain' -Body $Sql -TimeoutSec 15
    } catch {
        Fail "REST request failed: $($_.Exception.Message)"
    }
    if ($null -ne $response.code -and [int]$response.code -ne 0) {
        Fail "TDengine error $($response.code): $($response.desc)"
    }
    return $response
}

Write-Host "TDengine REST smoke test"
Write-Host "  url:      $base"
Write-Host "  database: $Database"
Write-Host "  vehicle:  $VehicleId"

$version = Invoke-TaosSql 'SELECT SERVER_VERSION()'
Ok "server version query returned code=$($version.code)"

Invoke-TaosSql "CREATE DATABASE IF NOT EXISTS ``$Database``" | Out-Null
Ok "database $Database ready"

Invoke-TaosSql "CREATE STABLE IF NOT EXISTS ``$Database``.``telemetry`` (ts TIMESTAMP, lat DOUBLE, lon DOUBLE, speed DOUBLE, heading DOUBLE, ignition BOOL, soc DOUBLE, odometer DOUBLE) TAGS (vehicle_id NCHAR(64))" | Out-Null
Ok 'super table telemetry ready'

$ts = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
$safeId = $VehicleId.Replace("'", "''")
$insert = "INSERT INTO ``$Database``.``v_smoke`` USING ``$Database``.``telemetry`` TAGS ('$safeId') VALUES ($ts, 31.2, 121.4, 50, 90, true, 80, 100)"
Invoke-TaosSql $insert | Out-Null
Ok 'sample inserted'

$count = Invoke-TaosSql "SELECT COUNT(*) FROM ``$Database``.``telemetry`` WHERE vehicle_id = '$safeId'"
if ($null -eq $count.data -or $count.data.Count -eq 0) {
    Fail 'count query returned no data'
}
Ok "count query returned: $($count.data[0][0])"

Ok 'TDengine smoke test complete'
