param(
    [int]$Viewers = 1000,
    [int]$Seconds = 3,
    [int]$Duration = 45
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

if (-not (Test-Path "target\release\m7s-rs.exe")) {
    throw "target\release\m7s-rs.exe not found; run cargo build --release"
}

$server = Start-Process -FilePath "target\release\m7s-rs.exe" `
    -ArgumentList "--config config\server.toml --demo --duration $Duration" `
    -PassThru -RedirectStandardOutput "bench-server.out" -RedirectStandardError "bench-server.err"

try {
    Start-Sleep -Seconds 2
    $before = Get-Process -Id $server.Id
    $cpuBefore = $before.CPU
    $start = Get-Date

    $loadOutput = node scripts\load_test.cjs $Viewers $Seconds | Out-String
    Write-Host $loadOutput.Trim()

    $after = Get-Process -Id $server.Id
    $api = Invoke-RestMethod -Uri "http://127.0.0.1:8180/api/streams" -ErrorAction SilentlyContinue
    $stream = $api.data.streams | Where-Object { $_.key -eq "live/test" } | Select-Object -First 1
    $p99 = if ($stream) { [int64]$stream.latency_p99_us } else { 0 }
    $samples = if ($stream) { [int64]$stream.latency_samples } else { 0 }
    $elapsed = ((Get-Date) - $start).TotalSeconds
    [pscustomobject]@{
        viewers = $Viewers
        ok = if ($loadOutput -match 'ok=(\d+)') { [int]$Matches[1] } else { 0 }
        failed = if ($loadOutput -match 'failed=(\d+)') { [int]$Matches[1] } else { 0 }
        seconds = $Seconds
        elapsed_sec = [math]::Round($elapsed, 2)
        working_set_mb = [math]::Round($after.WorkingSet64 / 1MB, 2)
        private_mb = [math]::Round($after.PrivateMemorySize64 / 1MB, 2)
        cpu_sec = [math]::Round($after.CPU - $cpuBefore, 2)
        cpu_pct_avg = if ($elapsed -gt 0) { [math]::Round((($after.CPU - $cpuBefore) / $elapsed) * 100, 2) } else { 0 }
        latency_p99_us = $p99
        latency_samples = $samples
    } | ConvertTo-Json
} finally {
    if (-not $server.HasExited) {
        $server.WaitForExit()
    }
}
