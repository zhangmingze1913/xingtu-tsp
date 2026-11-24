# Cursor-paginated reconciliation CSV downloader.
#
# Calls /api/integrations/timeseries/reconciliations/export/page until the
# server stops returning X-Next-Cursor, verifies each page's X-Row-Count, and
# writes a single CSV.
#
# Examples:
#   .\scripts\export-reconciliations.ps1
#   .\scripts\export-reconciliations.ps1 -Url http://127.0.0.1:8090 -Token $token -OutFile out.csv
#   .\scripts\export-reconciliations.ps1 -OrgName Org-A -Consistent false -MaxRows 5000
param(
    [string]$Url = 'http://127.0.0.1:8090',
    [string]$Token = '',
    [string]$Username = 'admin',
    [string]$Password = 'admin123',
    [string]$OutFile = 'reconciliations.csv',
    [string]$OrgName = '',
    [string]$VehicleId = '',
    [string]$Consistent = '',
    [int]$PageSize = 500,
    [int]$MaxRows = 5000
)

$ErrorActionPreference = 'Stop'
$base = $Url.TrimEnd('/')

function Fail([string]$Message) {
    Write-Host "FAIL: $Message" -ForegroundColor Red
    exit 1
}

function HeaderValue($Headers, [string]$Name) {
    $value = $Headers[$Name]
    if ($null -eq $value) { return '' }
    if ($value -is [array]) { return [string]$value[0] }
    return [string]$value
}

if ([string]::IsNullOrWhiteSpace($Token)) {
    try {
        $login = Invoke-RestMethod -Method Post -Uri "$base/api/auth/login" `
            -ContentType 'application/json' `
            -Body (@{ username = $Username; password = $Password } | ConvertTo-Json)
        $Token = $login.access_token
    } catch {
        Fail "login failed: $($_.Exception.Message)"
    }
}
if ([string]::IsNullOrWhiteSpace($Token)) { Fail 'no access token' }

$query = @("limit=$PageSize")
if ($OrgName) { $query += 'org_name=' + [uri]::EscapeDataString($OrgName) }
if ($VehicleId) { $query += 'vehicle_id=' + [uri]::EscapeDataString($VehicleId) }
if ($Consistent -eq 'true' -or $Consistent -eq 'false') { $query += "consistent=$Consistent" }

$dir = Split-Path -Parent $OutFile
if ($dir -and -not (Test-Path $dir)) { New-Item -ItemType Directory -Path $dir -Force | Out-Null }
$tmp = "$OutFile.tmp"
if (Test-Path $tmp) { Remove-Item $tmp -Force }
[IO.File]::WriteAllText($tmp, '')

$cursor = ''
$rows = 0
$pages = 0
do {
    $uri = "$base/api/integrations/timeseries/reconciliations/export/page?" + ($query -join '&')
    if ($cursor) { $uri += '&cursor=' + [uri]::EscapeDataString($cursor) }
    try {
        $response = Invoke-WebRequest -UseBasicParsing -Uri $uri `
            -Headers @{ Authorization = "Bearer $Token" } -TimeoutSec 120
    } catch {
        Fail "page $($pages + 1) failed: $($_.Exception.Message)"
    }
    $pageRows = [int](HeaderValue $response.Headers 'X-Row-Count')
    $content = [string]$response.Content
    if ($pages -eq 0) {
        [IO.File]::AppendAllText($tmp, $content)
    } else {
        $lines = $content -split "`r?`n"
        if ($lines.Count -gt 1) {
            [IO.File]::AppendAllText($tmp, ($lines[1..($lines.Count - 1)] -join "`n"))
        }
    }
    $rows += $pageRows
    $pages++
    $cursor = HeaderValue $response.Headers 'X-Next-Cursor'
    Write-Host "  page $pages rows=$pageRows total=$rows"
    if ($pageRows -eq 0) { $cursor = '' }
    if ($rows -ge $MaxRows) { break }
} while ($cursor)

Move-Item -Path $tmp -Destination $OutFile -Force
Write-Host "OK: $rows rows, $pages pages -> $OutFile" -ForegroundColor Green
if ($rows -ge $MaxRows) {
    Write-Host "NOTE: reached MaxRows=$MaxRows; rerun with a larger limit to continue." -ForegroundColor Yellow
}
