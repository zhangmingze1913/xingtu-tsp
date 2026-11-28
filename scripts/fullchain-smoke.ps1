param(
    [string]$BaseUrl = "http://127.0.0.1:8090",
    [string]$Jt808Addr = "127.0.0.1:18080",
    [string]$Phone = "013800000001",
    [string]$Auth = "XT000001",
    [int]$Sequential = 100,
    [int]$Concurrency = 20,
    [int]$PerWorker = 10
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$sim = Join-Path $root "target\debug\jt808_sim.exe"
$bench = Join-Path $PSScriptRoot "bench_api.ps1"
if (-not (Test-Path $sim)) {
    throw "missing $sim; run: cargo build --bin jt808_sim"
}

$out = Join-Path $env:TEMP "tsp-fullchain-sim.out"
$err = Join-Path $env:TEMP "tsp-fullchain-sim.err"
Remove-Item $out, $err -Force -ErrorAction SilentlyContinue

$oldAddr = $env:TSP_JT808_ADDR
$oldPhone = $env:TSP_JT808_PHONE
$oldAuth = $env:TSP_JT808_AUTH
$env:TSP_JT808_ADDR = $Jt808Addr
$env:TSP_JT808_PHONE = $Phone
$env:TSP_JT808_AUTH = $Auth
try {
    $simProcess = Start-Process -FilePath $sim `
        -RedirectStandardOutput $out -RedirectStandardError $err `
        -WindowStyle Hidden -PassThru
    & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $bench `
        -BaseUrl $BaseUrl -Sequential $Sequential `
        -Concurrency $Concurrency -PerWorker $PerWorker
    if (-not $simProcess.WaitForExit(20000)) {
        throw "JT808 simulator did not finish in 20s"
    }
} finally {
    $env:TSP_JT808_ADDR = $oldAddr
    $env:TSP_JT808_PHONE = $oldPhone
    $env:TSP_JT808_AUTH = $oldAuth
}

$stdout = if (Test-Path $out) { [string](Get-Content $out -Raw) } else { "" }
$stderr = if (Test-Path $err) { [string](Get-Content $err -Raw) } else { "" }
$stderrText = [string]$stderr
if ($stdout -match "(?m)^\s*NG" -or $stdout -match "result=[1-9]" -or $stderrText.Trim().Length -gt 0) {
    Write-Host $stdout
    Write-Host $stderr
    throw "JT808 full-chain smoke failed"
}
Write-Host "full-chain smoke passed: HTTP + JT808 auth/location"
