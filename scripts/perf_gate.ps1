param(
    [int]$Viewers = 5000,
    [int]$Seconds = 8,
    [int]$Duration = 25,
    [double]$MaxCpuPct = 120,
    [double]$MaxP99Ms = 250,
    [double]$MaxMemMb = 64
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

Write-Host "perf gate: viewers=$Viewers seconds=$Seconds"
$raw = & "$PSScriptRoot\bench.ps1" -Viewers $Viewers -Seconds $Seconds -Duration $Duration | Out-String
# bench.ps1 prints one JSON object at the end; keep from the first standalone brace.
$start = $raw.IndexOf("{")
if ($start -lt 0) {
    Write-Error "bench output has no JSON:`n$raw"
    exit 2
}
$jsonText = $raw.Substring($start).Trim()
$result = $jsonText | ConvertFrom-Json
$result | ConvertTo-Json | Set-Content "bench-last.json"

$p99Ms = [math]::Round($result.latency_p99_us / 1000.0, 2)
$fail = @()
if ($result.ok -ne $result.viewers) { $fail += "success $($result.ok)/$($result.viewers)" }
if ($result.cpu_pct_avg -gt $MaxCpuPct) { $fail += "cpu $($result.cpu_pct_avg)% > $MaxCpuPct%" }
if ($p99Ms -gt $MaxP99Ms) { $fail += "p99 ${p99Ms}ms > ${MaxP99Ms}ms" }
if ($result.working_set_mb -gt $MaxMemMb) { $fail += "mem $($result.working_set_mb)MB > $MaxMemMb MB" }

[pscustomobject]@{
    viewers = $result.viewers
    ok = $result.ok
    cpu_pct = $result.cpu_pct_avg
    p99_ms = $p99Ms
    mem_mb = $result.working_set_mb
    gate = if ($fail.Count -eq 0) { "PASS" } else { "FAIL: " + ($fail -join "; ") }
} | ConvertTo-Json

if ($fail.Count -gt 0) { exit 1 }
