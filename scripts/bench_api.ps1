# Minimal API baseline for the vehicle list hot path.
# Usage: pwsh -File scripts/bench_api.ps1 -BaseUrl http://127.0.0.1:8091

param(
    [string]$BaseUrl = "http://127.0.0.1:8091",
    [string]$Username = "admin",
    [string]$Password = "admin123",
    [int]$Sequential = 100,
    [int]$Concurrency = 10,
    [int]$PerWorker = 10
)

$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Net.Http

$login = Invoke-RestMethod -Method Post -Uri "$BaseUrl/api/auth/login" `
    -ContentType "application/json" `
    -Body (@{ username = $Username; password = $Password } | ConvertTo-Json)
$token = $login.access_token
$headers = @{ Authorization = "Bearer $token" }
$uri = "$BaseUrl/api/vehicles?page=1&page_size=50"

# Warmup
1..5 | ForEach-Object { Invoke-RestMethod -Uri $uri -Headers $headers | Out-Null }

$times = New-Object System.Collections.Generic.List[double]
for ($i = 0; $i -lt $Sequential; $i++) {
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    Invoke-RestMethod -Uri $uri -Headers $headers | Out-Null
    $sw.Stop()
    $times.Add($sw.Elapsed.TotalMilliseconds)
}
$sorted = $times | Sort-Object
$avg = ($times | Measure-Object -Average).Average
$p50 = $sorted[[int]([math]::Floor($sorted.Count * 0.50))]
$p95 = $sorted[[int]([math]::Floor($sorted.Count * 0.95))]
$max = ($times | Measure-Object -Maximum).Maximum

Write-Output ("sequential n={0} avg={1:N1}ms p50={2:N1}ms p95={3:N1}ms max={4:N1}ms" -f `
    $Sequential, $avg, $p50, $p95, $max)

$client = [System.Net.Http.HttpClient]::new()
$client.DefaultRequestHeaders.Authorization =
    [System.Net.Http.Headers.AuthenticationHeaderValue]::new("Bearer", $token)
$tasks = New-Object System.Collections.Generic.List[System.Threading.Tasks.Task]
for ($i = 0; $i -lt ($Concurrency * $PerWorker); $i++) {
    $tasks.Add($client.GetAsync($uri))
}
$sw = [System.Diagnostics.Stopwatch]::StartNew()
[System.Threading.Tasks.Task]::WaitAll($tasks.ToArray())
$sw.Stop()
$failed = ($tasks | Where-Object { -not $_.Result.IsSuccessStatusCode }).Count
$client.Dispose()
$total = $Concurrency * $PerWorker
Write-Output ("concurrent n={0} workers={1} elapsed={2:N2}s throughput={3:N1} req/s failed={4}" -f `
    $total, $Concurrency, $sw.Elapsed.TotalSeconds, ($total / $sw.Elapsed.TotalSeconds), $failed)
