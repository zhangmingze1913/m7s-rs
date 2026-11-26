param(
    [int]$Seconds = 10,
    [int]$Concurrency = 20,
    [string]$Config = "config/server.toml"
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$server = Start-Process -FilePath "target\release\m7s-rs.exe" `
    -ArgumentList "--config $Config --demo --duration 120" `
    -PassThru -RedirectStandardOutput "hls-bench.out" -RedirectStandardError "hls-bench.err"
try {
    Start-Sleep -Seconds 3
    $before = Get-Process -Id $server.Id
    $cpu0 = $before.CPU
    $start = Get-Date
    $out = node scripts\hls_bench.cjs $Seconds $Concurrency 8180 | Out-String
    Write-Host $out.Trim()
    $after = Get-Process -Id $server.Id
    $result = $out.Trim() | ConvertFrom-Json
    $result | Add-Member -NotePropertyName cpu_pct -NotePropertyValue ([math]::Round((($after.CPU - $cpu0) / [math]::Max(0.1, ((Get-Date) - $start).TotalSeconds)) * 100, 2))
    $result | Add-Member -NotePropertyName mem_mb -NotePropertyValue ([math]::Round($after.WorkingSet64 / 1MB, 2))
    $result | ConvertTo-Json
} finally {
    if (-not $server.HasExited) { $server.Kill() }
}
