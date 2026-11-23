param(
    [Parameter(Mandatory = $true)][string]$Backup,
    [string]$PgBin = "C:\Users\Administrator\Downloads\pgsql\pgsql\bin",
    [string]$HostName = "127.0.0.1",
    [int]$Port = 55432,
    [string]$User = "tsp",
    [string]$Database = "tsp",
    [switch]$VerifyOnly,
    [switch]$Custom
)

$ErrorActionPreference = "Stop"
if (-not (Test-Path -LiteralPath $Backup)) { throw "backup not found: $Backup" }

$manifestPath = "$Backup.manifest.json"
if (Test-Path -LiteralPath $manifestPath) {
    $manifest = Get-Content -Raw -LiteralPath $manifestPath | ConvertFrom-Json
    $actual = (Get-FileHash -Algorithm SHA256 -Path $Backup).Hash.ToLowerInvariant()
    if ($manifest.sha256 -ne $actual) { throw "backup checksum mismatch" }
    Write-Output "checksum=ok"
} else {
    Write-Output "checksum=missing-manifest"
}

if ($VerifyOnly) { exit 0 }

if ($Custom) {
    & (Join-Path $PgBin "pg_restore.exe") -h $HostName -p "$Port" -U $User -d $Database --no-owner --clean --if-exists $Backup
} else {
    & (Join-Path $PgBin "psql.exe") -h $HostName -p "$Port" -U $User -d $Database -v ON_ERROR_STOP=1 -f $Backup
}
if ($LASTEXITCODE -ne 0) { throw "restore failed with exit code $LASTEXITCODE" }
Write-Output "restore=ok"
