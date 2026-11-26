#requires -Version 5.1
<#
.SYNOPSIS
    Initialize a local Xingtu TSP configuration and secrets.

.DESCRIPTION
    Generates .env, JWT/JT808 keys, PII/export secrets and the SQLite data
    directory. It does not start databases or install Rust; use -Build/-Run to
    build or launch after initialization.

.EXAMPLES
    .\scripts\init-system.ps1
    .\scripts\init-system.ps1 -Database postgres -DatabaseUrl 'postgres://tsp:...@127.0.0.1:5432/tsp?sslmode=disable'
    .\scripts\init-system.ps1 -Production -AdminPassword 'use-a-secret-manager' -Build
    .\scripts\init-system.ps1 -Force -EnvFile .env.local
#>
[CmdletBinding()]
param(
    [ValidateSet('sqlite', 'postgres', 'mysql', 'kingbase')]
    [string]$Database = 'sqlite',

    [string]$DatabaseUrl = '',

    [string]$EnvFile = '.env',

    [string]$DataDir = 'data',

    [switch]$Force,

    [switch]$Production,

    [switch]$Build,

    [switch]$Run,

    [string]$AdminPassword = ''
)

$ErrorActionPreference = 'Stop'
$RepoRoot = Split-Path -Parent $PSScriptRoot

function Resolve-RepoPath([string]$Path) {
    if ([System.IO.Path]::IsPathRooted($Path)) {
        return [System.IO.Path]::GetFullPath($Path)
    }
    return [System.IO.Path]::GetFullPath((Join-Path $RepoRoot $Path))
}

function New-RandomBytes([int]$Count) {
    $bytes = New-Object byte[] $Count
    $rng = [System.Security.Cryptography.RandomNumberGenerator]::Create()
    try { $rng.GetBytes($bytes) } finally { $rng.Dispose() }
    return $bytes
}

function New-RandomHex([int]$Count) {
    return ((New-RandomBytes $Count) | ForEach-Object { $_.ToString('x2') }) -join ''
}

function New-RandomBase64([int]$Count) {
    return [Convert]::ToBase64String((New-RandomBytes $Count))
}

function Write-EnvLine([System.Collections.Generic.List[string]]$Lines, [string]$Key, [string]$Value) {
    $Lines.Add("$Key=$Value")
}

$envPath = Resolve-RepoPath $EnvFile
$dataPath = Resolve-RepoPath $DataDir
$secretDir = Join-Path $RepoRoot 'secrets'

if ((Test-Path $envPath) -and -not $Force) {
    throw "Refusing to overwrite $envPath; pass -Force to back it up and regenerate."
}
if (Test-Path $envPath) {
    $backup = "$envPath.bak." + (Get-Date -Format 'yyyyMMdd-HHmmss')
    Copy-Item $envPath $backup
    Write-Host "backup: $backup" -ForegroundColor DarkGray
}

New-Item -ItemType Directory -Force -Path $dataPath, $secretDir | Out-Null

if ([string]::IsNullOrWhiteSpace($DatabaseUrl)) {
    switch ($Database) {
        'sqlite' {
            $DatabaseUrl = (Join-Path $dataPath 'tsp.db').Replace('\', '/')
        }
        'postgres' {
            throw "PostgreSQL requires -DatabaseUrl, e.g. postgres://tsp:password@127.0.0.1:5432/tsp?sslmode=disable"
        }
        'mysql' {
            throw "MySQL requires -DatabaseUrl, e.g. mysql://tsp:password@127.0.0.1:3306/tsp"
        }
        'kingbase' {
            throw "KingbaseES requires -DatabaseUrl, e.g. kingbase://tsp:password@127.0.0.1:54321/tsp?sslmode=disable"
        }
    }
}

$postgis = if ($Database -eq 'postgres') { 'true' } else { 'false' }
if ($Production -and $Database -eq 'sqlite') {
    throw "Production mode requires PostgreSQL, KingbaseES or MySQL; SQLite is not supported."
}

$admin = $AdminPassword
if ([string]::IsNullOrWhiteSpace($admin)) {
    if ($Production) {
        $admin = New-RandomBase64 18
    } else {
        $admin = 'admin123'
    }
}

$jwtKeyFile = ''
$jwtPubFile = ''
$jt808Cert = ''
$jt808Key = ''
$openssl = Get-Command openssl -ErrorAction SilentlyContinue
if (-not $openssl) {
    $gitOpenSsl = 'C:\Program Files\Git\usr\bin\openssl.exe'
    if (Test-Path $gitOpenSsl) { $openssl = Get-Item $gitOpenSsl }
}
if ($openssl) {
    $opensslExe = if ($openssl -is [System.Management.Automation.ApplicationInfo]) {
        $openssl.Source
    } else {
        $openssl.FullName
    }
    $jwtKeyFile = Join-Path $secretDir 'jwt-private.pem'
    $jwtPubFile = Join-Path $secretDir 'jwt-public.pem'
    & $opensslExe genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:2048 -out $jwtKeyFile 2>$null
    if ($LASTEXITCODE -ne 0) { throw 'openssl genpkey failed' }
    & $opensslExe rsa -in $jwtKeyFile -pubout -out $jwtPubFile 2>$null
    if ($LASTEXITCODE -ne 0) { throw 'openssl rsa -pubout failed' }

    $jt808Key = Join-Path $secretDir 'jt808-key.pem'
    $jt808Cert = Join-Path $secretDir 'jt808-cert.pem'
    & $opensslExe genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:2048 -out $jt808Key 2>$null
    & $opensslExe req -new -x509 -key $jt808Key -out $jt808Cert -days 365 -subj '/CN=tsp-jt808' 2>$null
    if ($LASTEXITCODE -ne 0) { throw 'openssl self-signed cert failed' }
} elseif ($Production) {
    throw 'Production mode requires openssl on PATH to generate RS256/JT808 keys.'
} else {
    Write-Warning 'openssl not found; JWT will use process-local dev key.'
}

$lines = [System.Collections.Generic.List[string]]::new()
Write-EnvLine $lines 'TSP_ENV' $(if ($Production) { 'production' } else { 'development' })
Write-EnvLine $lines 'TSP_BIND' '127.0.0.1:8090'
Write-EnvLine $lines 'TSP_DATABASE' $DatabaseUrl
Write-EnvLine $lines 'TSP_WEB_DIR' 'web'
Write-EnvLine $lines 'TSP_DEMO_MODE' $(if ($Production) { 'false' } else { 'true' })
Write-EnvLine $lines 'TSP_CLUSTER_MODE' 'false'
Write-EnvLine $lines 'TSP_TIMESERIES_BACKEND' 'postgres'
Write-EnvLine $lines 'TSP_POSTGIS_ENABLED' $postgis
Write-EnvLine $lines 'TSP_JT808_ALLOW_PLAINTEXT' $(if ($Production) { 'false' } else { 'true' })
Write-EnvLine $lines 'TSP_JT808_ADDR' '127.0.0.1:18080'
Write-EnvLine $lines 'TSP_JWT_SECRET' (New-RandomHex 32)
Write-EnvLine $lines 'TSP_JWT_KEY_ID' 'tsp-rs256-1'
Write-EnvLine $lines 'TSP_ADMIN_PASSWORD' $admin
Write-EnvLine $lines 'TSP_PII_KEY' (New-RandomBase64 32)
Write-EnvLine $lines 'TSP_EXPORT_TOKEN_SECRET' (New-RandomHex 32)
Write-EnvLine $lines 'TSP_EXPORT_ENCRYPTION_KEY' (New-RandomBase64 32)
Write-EnvLine $lines 'TSP_EXPORT_ENCRYPTION_KEY_ID' 'current'
Write-EnvLine $lines 'TSP_EXPORT_REQUIRE_ENCRYPTION' 'true'
Write-EnvLine $lines 'RUST_LOG' 'xingtu_tsp=info'

if ($jwtKeyFile) {
    Write-EnvLine $lines 'TSP_JWT_PRIVATE_KEY_FILE' ($jwtKeyFile.Replace('\', '/'))
    Write-EnvLine $lines 'TSP_JWT_PUBLIC_KEY_FILE' ($jwtPubFile.Replace('\', '/'))
    Write-EnvLine $lines 'TSP_JT808_TLS_CERT' ($jt808Cert.Replace('\', '/'))
    Write-EnvLine $lines 'TSP_JT808_TLS_KEY' ($jt808Key.Replace('\', '/'))
}

[System.IO.File]::WriteAllLines($envPath, $lines, [System.Text.UTF8Encoding]::new($false))

if (-not $IsWindows -and (Get-Command chmod -ErrorAction SilentlyContinue)) {
    & chmod 600 $envPath
    & chmod 700 $secretDir
}

Write-Host ''
Write-Host 'TSP initialization complete' -ForegroundColor Green
Write-Host "  env:      $envPath"
Write-Host "  data:     $dataPath"
Write-Host "  database: $Database"
if ($Production) {
    Write-Host "  admin:    admin / $admin" -ForegroundColor Yellow
    Write-Host '  Store this password in a secret manager; it will not be shown again.'
} else {
    Write-Host '  demo:     admin / admin123'
}
Write-Host "  secrets:  $secretDir"
Write-Host ''
Write-Host 'Next:'
Write-Host '  cargo run --release'
Write-Host '  curl http://127.0.0.1:8090/api/health'

if ($Build -or $Run) {
    Push-Location $RepoRoot
    try {
        if ($Build) {
            Write-Host ''
            Write-Host 'Building release...' -ForegroundColor Cyan
            & cargo build --release
            if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }
        }
        if ($Run) {
            foreach ($line in Get-Content $envPath) {
                if ($line -match '^([A-Za-z_][A-Za-z0-9_]*)=(.*)$') {
                    [Environment]::SetEnvironmentVariable($Matches[1], $Matches[2], 'Process')
                }
            }
            Write-Host 'Starting TSP...' -ForegroundColor Cyan
            & cargo run --release
        }
    } finally {
        Pop-Location
    }
}
