# RustFS S3 smoke test for Windows PowerShell 5.1+.
#
# Prerequisites:
#   - RustFS running (docker compose up -d rustfs)
#   - AWS CLI v2 on PATH (aws --version)
#
# Examples:
#   .\scripts\rustfs-smoke.ps1
#   .\scripts\rustfs-smoke.ps1 -Endpoint http://127.0.0.1:9000 -Bucket tsp-media
#   $env:RUSTFS_ACCESS_KEY='...'; $env:RUSTFS_SECRET_KEY='...'; .\scripts\rustfs-smoke.ps1
param(
    [string]$Endpoint = 'http://127.0.0.1:9000',
    [string]$Bucket = 'tsp-media',
    [string]$AccessKey = $(if ($env:RUSTFS_ACCESS_KEY) { $env:RUSTFS_ACCESS_KEY } else { 'rustfsadmin' }),
    [string]$SecretKey = $(if ($env:RUSTFS_SECRET_KEY) { $env:RUSTFS_SECRET_KEY } else { 'rustfsadmin' }),
    [string]$Sample = 'assets/sample.h264',
    [string]$Key = 'smoke/sample.h264',
    [switch]$SkipUpload
)

$ErrorActionPreference = 'Stop'
$endpoint = $Endpoint.TrimEnd('/')
$repoRoot = Split-Path -Parent $PSScriptRoot
$samplePath = if ([System.IO.Path]::IsPathRooted($Sample)) { $Sample } else { Join-Path $repoRoot $Sample }

function Fail([string]$Message) {
    Write-Host "FAIL: $Message" -ForegroundColor Red
    exit 1
}

function Ok([string]$Message) {
    Write-Host "OK:   $Message" -ForegroundColor Green
}

Write-Host "RustFS smoke test"
Write-Host "  endpoint: $endpoint"
Write-Host "  bucket:   $Bucket"
Write-Host "  sample:   $samplePath"

# 1. Health endpoints. Console may be disabled, so only warn on 9001.
try {
    $health = Invoke-WebRequest -UseBasicParsing -Uri "$endpoint/health" -TimeoutSec 10
    if ($health.StatusCode -ne 200) { Fail "S3 health returned $($health.StatusCode)" }
    Ok 'S3 /health'
} catch {
    Fail "S3 /health unreachable: $($_.Exception.Message)"
}

$console = ($endpoint -replace ':9000$', ':9001')
try {
    $consoleHealth = Invoke-WebRequest -UseBasicParsing -Uri "$console/rustfs/console/health" -TimeoutSec 5
    if ($consoleHealth.StatusCode -eq 200) { Ok 'console /rustfs/console/health' }
} catch {
    Write-Host "WARN: console health unavailable at $console (console may be disabled)" -ForegroundColor Yellow
}

# 2. AWS CLI.
$aws = Get-Command aws -ErrorAction SilentlyContinue
if (-not $aws) {
    Fail 'AWS CLI not found. Install AWS CLI v2 and retry: https://docs.aws.amazon.com/cli/'
}
$env:AWS_ACCESS_KEY_ID = $AccessKey
$env:AWS_SECRET_ACCESS_KEY = $SecretKey
$env:AWS_DEFAULT_REGION = 'us-east-1'
$env:AWS_EC2_METADATA_DISABLED = 'true'

# 3. Bucket.
& aws --endpoint-url $endpoint s3api head-bucket --bucket $Bucket 2>$null | Out-Null
if ($LASTEXITCODE -ne 0) {
    & aws --endpoint-url $endpoint s3 mb "s3://$Bucket" | Out-Null
    if ($LASTEXITCODE -ne 0) { Fail "could not create bucket $Bucket" }
    Ok "created bucket $Bucket"
} else {
    Ok "bucket $Bucket exists"
}

if ($SkipUpload) {
    Ok 'skip upload requested'
    exit 0
}

if (-not (Test-Path -LiteralPath $samplePath)) {
    Fail "sample not found: $samplePath"
}

# 4. Upload, list, download, compare.
& aws --endpoint-url $endpoint s3 cp $samplePath "s3://$Bucket/$Key" --only-show-errors
if ($LASTEXITCODE -ne 0) { Fail 'upload failed' }
Ok "uploaded s3://$Bucket/$Key"

$listed = & aws --endpoint-url $endpoint s3 ls "s3://$Bucket/smoke/" 2>$null
if ($LASTEXITCODE -ne 0 -or -not ($listed -match 'sample\.h264')) {
    Fail 'uploaded object not visible in listing'
}
Ok 'listed uploaded object'

$temp = Join-Path ([System.IO.Path]::GetTempPath()) ("rustfs-smoke-" + [guid]::NewGuid().ToString('N') + '.h264')
try {
    & aws --endpoint-url $endpoint s3 cp "s3://$Bucket/$Key" $temp --only-show-errors
    if ($LASTEXITCODE -ne 0) { Fail 'download failed' }
    $sourceHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $samplePath).Hash
    $downloadHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $temp).Hash
    if ($sourceHash -ne $downloadHash) { Fail 'downloaded object hash does not match source' }
    Ok "downloaded object hash matches ($($sourceHash.Substring(0, 12))...)"
} finally {
    Remove-Item -LiteralPath $temp -Force -ErrorAction SilentlyContinue
}

Ok 'RustFS smoke test complete'
