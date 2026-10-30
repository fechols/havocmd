param([string]$Distribution='Ubuntu-24.04', [switch]$SkipEncoding)
$ErrorActionPreference='Stop'
$projectRoot=(Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
Push-Location $projectRoot
try {
    cargo test --offline
    if ($LASTEXITCODE) { throw 'Host tests failed.' }
    if (!$SkipEncoding) {
        & powershell.exe -NoProfile -ExecutionPolicy Bypass -File tools/import_music.ps1 -FullCatalog -StageOnly -Jobs 3 -Resume -OutputDirectory target/music/full16
        if ($LASTEXITCODE) { throw 'Full music encoding failed.' }
    }
    cargo run --offline --release -- bake-art
    if ($LASTEXITCODE) { throw 'Artwork bake failed.' }
    $linuxRoot=(& wsl -d $Distribution -- wslpath -a -u $projectRoot).Trim()
    if ($LASTEXITCODE) { throw 'WSL unavailable.' }
    & wsl -d $Distribution -- python3 "$linuxRoot/tools/prepare-musicdisk.py" --music "$linuxRoot/target/music/full16" --packer "$linuxRoot/target/release/havocmd.exe"
    if ($LASTEXITCODE) { throw 'Full content verification and packaging failed.' }
    & wsl -d $Distribution -- bash "$linuxRoot/scripts/build-musicdisk.sh"
    if ($LASTEXITCODE) { throw 'C64 build failed.' }
    & wsl -d $Distribution -- python3 "$linuxRoot/tools/test-musicdisk.py"
    if ($LASTEXITCODE) { throw 'Offline production tests failed.' }
    & wsl -d $Distribution -- python3 "$linuxRoot/tools/finalize-musicdisk.py"
    if ($LASTEXITCODE) { throw 'Bundle finalization failed.' }
    Write-Output 'Full 16 kHz stereo musicdisk ready in dist/havocmd. Physical hardware validation pending.'
} finally { Pop-Location }
