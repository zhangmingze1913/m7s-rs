$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root
$cargo = "C:\Users\Administrator\.cargo\bin\cargo.exe"
function Step($name, $block) {
  Write-Host "== $name =="
  & $block
  if ($LASTEXITCODE -ne 0) { throw "$name failed" }
}
Step "clippy" { rustup run 1.94-x86_64-pc-windows-msvc cargo clippy --offline --workspace --all-targets -- -D warnings }
Step "tests" { rustup run 1.94-x86_64-pc-windows-msvc cargo test --offline --workspace }
Step "release" { rustup run 1.94-x86_64-pc-windows-msvc cargo build --release --offline --workspace }
Write-Host "== sdk =="
node web\sdk_test.cjs
if ($LASTEXITCODE -ne 0) { throw "sdk test failed" }
Write-Host "== react admin =="
Push-Location web\admin
npm run build | Out-Null
node test.cjs
if ($LASTEXITCODE -ne 0) { Pop-Location; throw "admin test failed" }
Pop-Location
Write-Host "ACCEPTANCE_OK"
