param(
    [int]$Streams = 100,
    [int]$Seconds = 8,
    [int]$Workers = 4,
    [string]$Config = "config/server.toml",
    [string]$Sample = "bench-sample.flv"
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root
$ffmpeg = if ($env:M7S_FFMPEG) { $env:M7S_FFMPEG } else { "node_modules/ffmpeg-static/ffmpeg.exe" }

if (-not (Test-Path $Sample)) {
    Write-Host "generating sample with $ffmpeg"
    & $ffmpeg -y -hide_banner -loglevel error `
        -f lavfi -i "testsrc2=size=640x360:rate=25" `
        -f lavfi -i "sine=frequency=440:sample_rate=44100" `
        -t 8 -c:v libx264 -preset ultrafast -pix_fmt yuv420p -g 25 -c:a aac -f flv $Sample
    if ($LASTEXITCODE -ne 0) { throw "ffmpeg sample generation failed" }
}

$server = Start-Process -FilePath "target\release\m7s-rs.exe" `
    -ArgumentList "--config $Config --duration 180" `
    -PassThru -RedirectStandardOutput "rtmp-bench.out" -RedirectStandardError "rtmp-bench.err"
try {
    Start-Sleep -Seconds 2
    $before = Get-Process -Id $server.Id
    $cpu0 = $before.CPU
    $start = Get-Date
    $out = node scripts\multi_push.cjs $Streams $Seconds 127.0.0.1:1935 $Workers $Sample | Out-String
    Write-Host $out.Trim()
    Start-Sleep -Seconds 2
    $after = Get-Process -Id $server.Id
    $api = Invoke-RestMethod -Uri "http://127.0.0.1:8180/api/streams"
    $active = @($api.data.streams | Where-Object { $_.app -eq "live" }).Count
    [pscustomobject]@{
        streams = $Streams
        active = $active
        elapsed_sec = [math]::Round(((Get-Date) - $start).TotalSeconds, 2)
        cpu_pct = [math]::Round((($after.CPU - $cpu0) / [math]::Max(0.1, ((Get-Date) - $start).TotalSeconds)) * 100, 2)
        mem_mb = [math]::Round($after.WorkingSet64 / 1MB, 2)
    } | ConvertTo-Json
} finally {
    if (-not $server.HasExited) { $server.Kill() }
}
