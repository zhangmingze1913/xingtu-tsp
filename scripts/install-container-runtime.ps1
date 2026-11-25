# Install Docker Desktop prerequisites on Windows 10/11 (admin).
#
# This script enables WSL2/VirtualMachinePlatform, installs the WSL2 kernel,
# sets WSL default version 2, and runs the Docker Desktop installer.
#
# A REBOOT is required after enabling the Windows features. The script does not
# reboot automatically.
#
# Run in an elevated PowerShell:
#   .\scripts\install-container-runtime.ps1
#   .\scripts\install-container-runtime.ps1 -SkipDocker
param(
    [switch]$SkipDocker,
    [string]$DockerInstaller = "$env:TEMP\DockerDesktopInstaller.exe"
)

$ErrorActionPreference = 'Stop'

function Fail([string]$Message) {
    Write-Host "FAIL: $Message" -ForegroundColor Red
    exit 1
}

$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$principal = New-Object Security.Principal.WindowsPrincipal($identity)
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    Fail 'run this script in an elevated PowerShell (Run as Administrator)'
}

if (Get-Command docker -ErrorAction SilentlyContinue) {
    Write-Host 'Docker is already installed; nothing to do.'
    exit 0
}

Write-Host 'Enabling WSL and VirtualMachinePlatform (no restart)...'
foreach ($feature in @('Microsoft-Windows-Subsystem-Linux', 'VirtualMachinePlatform')) {
    $state = (Get-WindowsOptionalFeature -Online -FeatureName $feature).State
    if ($state -ne 'Enabled') {
        & dism.exe /online /enable-feature /featurename:$feature /all /norestart | Out-Host
        # 3010 = success, reboot required.
        if ($LASTEXITCODE -ne 0 -and $LASTEXITCODE -ne 3010) {
            Fail "failed to enable $feature (dism exit $LASTEXITCODE)"
        }
    }
}

$kernel = Join-Path $env:TEMP 'wsl_update_x64.msi'
if (-not (Test-Path $kernel)) {
    Write-Host 'Downloading WSL2 kernel update...'
    Invoke-WebRequest -Uri 'https://wslstorestorage.blob.core.windows.net/wslblob/wsl_update_x64.msi' `
        -OutFile $kernel -UseBasicParsing
}
Write-Host 'Installing WSL2 kernel update...'
$p = Start-Process msiexec.exe -ArgumentList @('/i', $kernel, '/qn', '/norestart') -Wait -PassThru
if ($p.ExitCode -ne 0) { Fail "WSL2 kernel installer exited $($p.ExitCode)" }

& wsl.exe --set-default-version 2 | Out-Host

if (-not $SkipDocker) {
    if (-not (Test-Path $DockerInstaller)) {
        Write-Host 'Downloading Docker Desktop installer...'
        Invoke-WebRequest -Uri 'https://desktop.docker.com/win/main/amd64/Docker%20Desktop%20Installer.exe' `
            -OutFile $DockerInstaller -UseBasicParsing
    }
    Write-Host 'Installing Docker Desktop (this may take several minutes)...'
    $p = Start-Process $DockerInstaller -ArgumentList @('install', '--quiet', '--accept-license') -Wait -PassThru
    if ($p.ExitCode -ne 0) { Fail "Docker Desktop installer exited $($p.ExitCode)" }
}

Write-Host ''
Write-Host 'OK: prerequisites installed.' -ForegroundColor Green
Write-Host 'REBOOT WINDOWS, then start Docker Desktop and run:' -ForegroundColor Yellow
Write-Host '  docker version'
Write-Host '  .\scripts\kingbase-container.ps1 -EnvFile deploy\kingbase\.env'
Write-Host ''
Write-Host 'A licensed KingbaseES image and license.dat are still required.'
