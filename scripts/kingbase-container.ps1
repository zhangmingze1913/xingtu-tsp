# Start/stop a licensed KingbaseES container.
#
# Example:
#   .\scripts\kingbase-container.ps1 -EnvFile deploy\kingbase\.env
#   .\scripts\kingbase-container.ps1 -EnvFile deploy\kingbase\.env -Down
param(
    [string]$EnvFile = 'deploy\kingbase\.env',
    [string]$ComposeFile = 'deploy\kingbase\docker-compose.yml',
    [int]$Port = 54321,
    [int]$WaitSeconds = 180,
    [switch]$Down
)

$ErrorActionPreference = 'Stop'

function Fail([string]$Message) {
    Write-Host "FAIL: $Message" -ForegroundColor Red
    exit 1
}

$docker = Get-Command docker -ErrorAction SilentlyContinue
if (-not $docker) {
    Fail 'Docker is not installed. Install Docker/Podman or run on a host with a container runtime.'
}
if (-not (Test-Path $EnvFile)) {
    Fail "env file not found: $EnvFile (copy deploy/kingbase/.env.example)"
}

$compose = @('compose', '--env-file', $EnvFile, '-f', $ComposeFile)
$probe = & docker @compose version 2>&1
if ($LASTEXITCODE -ne 0) {
    $legacy = Get-Command docker-compose -ErrorAction SilentlyContinue
    if (-not $legacy) {
        Fail "docker compose plugin unavailable: $probe"
    }
    $compose = @('--env-file', $EnvFile, '-f', $ComposeFile)
    $dockerCompose = $legacy.Source
} else {
    $dockerCompose = $docker.Source
}

if ($Down) {
    & $dockerCompose @compose down
    if ($LASTEXITCODE -ne 0) { Fail 'docker compose down failed' }
    Write-Host 'KingbaseES container stopped.'
    exit 0
}

& $dockerCompose @compose up -d
if ($LASTEXITCODE -ne 0) { Fail 'docker compose up failed' }

$deadline = (Get-Date).AddSeconds($WaitSeconds)
while ((Get-Date) -lt $deadline) {
    try {
        $client = New-Object Net.Sockets.TcpClient
        $client.Connect('127.0.0.1', $Port)
        $client.Close()
        Write-Host "OK: KingbaseES is accepting TCP on 127.0.0.1:$Port" -ForegroundColor Green
        Write-Host 'Next: set TSP_DATABASE=kingbase://... and run the app.'
        exit 0
    } catch {
        Start-Sleep -Seconds 3
    }
}

Fail "KingbaseES did not open port $Port within $WaitSeconds seconds; check docker logs tsp-kingbase"
