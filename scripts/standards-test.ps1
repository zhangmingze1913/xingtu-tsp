# Standards conformance test entry point.
# Usage: pwsh -File scripts/standards-test.ps1
#        or: powershell -NoProfile -ExecutionPolicy Bypass -File scripts/standards-test.ps1

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Push-Location $root
try {
    Write-Host "== fmt =="
    cargo fmt --check
    if ($LASTEXITCODE -ne 0) { throw "cargo fmt --check failed" }

    Write-Host "== clippy =="
    cargo clippy --all-targets
    if ($LASTEXITCODE -ne 0) { throw "cargo clippy failed" }

    Write-Host "== standards + regression tests =="
    cargo test --lib `
        --test auth `
        --test vehicles `
        --test standards `
        --test storage `
        --test cluster `
        --test standards_conformance --test jt809_sim --test jt809_tls_sim --test gb32960_sim --test jt1078_sim --test pii_vault --test standards_fuzz
    if ($LASTEXITCODE -ne 0) { throw "standards tests failed" }

    Write-Host "== catalog counts =="
    Write-Host "See /api/integrations/standards catalog_counts and docs/standards-matrix.csv"
    Write-Host "OK"
}
finally {
    Pop-Location
}
