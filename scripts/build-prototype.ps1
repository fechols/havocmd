param([string]$Distribution='Ubuntu-24.04', [switch]$ReencodeFixture)
$ErrorActionPreference='Stop'
$projectRoot=(Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
Push-Location $projectRoot
try {
    cargo test --offline
    if ($LASTEXITCODE) { throw 'Host tests failed.' }
    if ($ReencodeFixture -or !(Test-Path -LiteralPath 'target/music/diagnostic/hammond-rocker.dlt')) {
        & powershell.exe -NoProfile -ExecutionPolicy Bypass -File tools/import_music.ps1
        if ($LASTEXITCODE) { throw 'Diagnostic encoding failed.' }
    }
    cargo run --offline --release -- bake-art assets/splashes target/art
    if ($LASTEXITCODE) { throw 'Artwork bake failed.' }
    $linuxRoot=(& wsl -d $Distribution -- wslpath -a -u $projectRoot).Trim()
    if ($LASTEXITCODE) { throw 'WSL unavailable.' }
    & wsl -d $Distribution -- python3 "$linuxRoot/tools/prepare-prototype.py"
    if ($LASTEXITCODE) { throw 'Fixture packaging failed.' }
    & wsl -d $Distribution -- bash "$linuxRoot/scripts/build-target.sh"
    if ($LASTEXITCODE) { throw 'C64 build failed.' }
    & wsl -d $Distribution -- python3 "$linuxRoot/tools/test-prototype.py"
    if ($LASTEXITCODE) { throw 'Linked machine-code validation failed.' }
    & wsl -d $Distribution -- python3 "$linuxRoot/tools/test-startup.py"
    if ($LASTEXITCODE) { throw 'Raw PRG startup validation failed.' }
    Copy-Item -LiteralPath assets/prototype.cfg -Destination dist/prototype/havocmd-prototype.cfg
    Copy-Item -LiteralPath target-c64/docs/prototype-readme.txt -Destination dist/prototype/README.txt
    Write-Output 'Prototype built. Hardware timing gate remains pending; see README.md.'
} finally { Pop-Location }
