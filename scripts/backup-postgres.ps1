param(
    [string]$OutDir = "backups",
    [string]$PgBin = "C:\Users\Administrator\Downloads\pgsql\pgsql\bin",
    [string]$HostName = "127.0.0.1",
    [int]$Port = 55432,
    [string]$User = "tsp",
    [string]$Database = "tsp",
    [switch]$SchemaOnly,
    [switch]$Custom
)

$ErrorActionPreference = "Stop"
$repo = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
$outPath = if ([System.IO.Path]::IsPathRooted($OutDir)) { $OutDir } else { Join-Path $repo $OutDir }
New-Item -ItemType Directory -Force -Path $outPath | Out-Null

$stamp = Get-Date -Format "yyyyMMdd-HHmmss"
$format = if ($Custom) { "custom" } else { "plain" }
$ext = if ($Custom) { "dump" } else { "sql" }
$sqlPath = Join-Path $outPath "tsp-$stamp.$ext"
$args = @("-h", $HostName, "-p", "$Port", "-U", $User, "-d", $Database, "--no-owner", "--format=$format", "-f", $sqlPath)
if ($SchemaOnly) { $args += "--schema-only" }

& (Join-Path $PgBin "pg_dump.exe") @args
if ($LASTEXITCODE -ne 0) { throw "pg_dump failed with exit code $LASTEXITCODE" }

$hash = (Get-FileHash -Algorithm SHA256 -Path $sqlPath).Hash.ToLowerInvariant()
$size = (Get-Item $sqlPath).Length
$manifest = [ordered]@{
    created_at = (Get-Date).ToUniversalTime().ToString("o")
    database = $Database
    schema_only = [bool]$SchemaOnly
    format = $format
    file = (Split-Path -Leaf $sqlPath)
    bytes = $size
    sha256 = $hash
}
$manifestPath = "$sqlPath.manifest.json"
$manifest | ConvertTo-Json | Set-Content -Encoding utf8 -Path $manifestPath
Write-Output "backup=$sqlPath bytes=$size sha256=$hash"
Write-Output "manifest=$manifestPath"
