[CmdletBinding()]
param(
    [string]$SourceDirectory, [string]$OutputDirectory, [string]$Encoder,
    [string]$DiagnosticTrack = 'hammond-rocker',
    [ValidateRange(0.1, 10.0)][double]$DiagnosticSeconds = 2.0,
    [switch]$FullCatalog, [switch]$StageOnly, [string]$TimingGateReport,
    [ValidateRange(1, 3)][int]$Jobs = 2, [switch]$Resume, [switch]$SkipBuild
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
function Get-Sha256([string]$Path) {
    $stream=[IO.File]::OpenRead($Path); $hasher=[Security.Cryptography.SHA256]::Create()
    try { return [BitConverter]::ToString($hasher.ComputeHash($stream)).Replace('-','').ToLowerInvariant() }
    finally { $hasher.Dispose(); $stream.Dispose() }
}
function Quote-Argument([string]$Value) {
    if ($Value.Contains('"')) { throw 'A process argument contains an unsupported quote.' }
    return '"'+$Value+'"'
}
$projectRoot=Split-Path -Parent $PSScriptRoot
$catalog=Get-Content -LiteralPath (Join-Path $projectRoot 'assets/music/catalog.json') -Raw | ConvertFrom-Json
$profile=[string]$catalog.profile; $sampleRate=[int]$catalog.sample_rate
if ($profile -notin @('stereo16k3','stereo24k3','stereo32k3') -or $catalog.channels -ne 2 -or $profile -ne ('stereo'+($sampleRate/1000)+'k3')) {
    throw 'The catalog must use a supported true-stereo DLT5 profile.'
}
if ($FullCatalog -and -not $StageOnly) {
    if (-not $TimingGateReport) { throw 'Use -StageOnly for offline preparation, or provide a real -TimingGateReport.' }
    $gate=Get-Content -LiteralPath $TimingGateReport -Raw | ConvertFrom-Json
    if ($gate.passed -ne $true -or $gate.hardware_measured -ne $true -or $gate.audio_profile -ne $profile) {
        throw "Timing gate must report passed=true, hardware_measured=true, audio_profile=$profile."
    }
}
$selected=@(if ($FullCatalog) { $catalog.tracks } else { $catalog.tracks | Where-Object name -EQ $DiagnosticTrack })
if (-not $FullCatalog -and $selected.Count -ne 1) { throw "Unknown diagnostic track: $DiagnosticTrack" }
if (-not $SourceDirectory) { $SourceDirectory=Join-Path $projectRoot $catalog.source_directory }
$SourceDirectory=(Resolve-Path -LiteralPath $SourceDirectory).Path
if (-not $OutputDirectory) {
    $OutputDirectory=Join-Path $projectRoot $(if ($FullCatalog) { "target/music/full$($sampleRate/1000)" } else { "target/music/diagnostic$($sampleRate/1000)" })
}
$OutputDirectory=[IO.Path]::GetFullPath($OutputDirectory)
New-Item -ItemType Directory -Force -Path $OutputDirectory | Out-Null
$buildDirectory=Join-Path $projectRoot 'target/music/encoder'
if (-not $Encoder) { $Encoder=Join-Path $buildDirectory 'release/encode-digital-music.exe' }
$Encoder=[IO.Path]::GetFullPath($Encoder)
if (-not $SkipBuild) {
    & cargo build --release --offline --manifest-path (Join-Path $PSScriptRoot 'encode-digital-music/Cargo.toml') --target-dir $buildDirectory
    if ($LASTEXITCODE -ne 0) { throw 'Encoder build failed.' }
}
if (-not (Test-Path -LiteralPath $Encoder -PathType Leaf)) { throw "Missing encoder: $Encoder" }
Get-Command ffmpeg,ffprobe -ErrorAction Stop | Out-Null
$ffmpegVersion=@(& ffmpeg -version)[0]
$encoderHash=Get-Sha256 $Encoder
$utf8=New-Object Text.UTF8Encoding($false)
$completed=New-Object 'System.Collections.Generic.List[object]'
$tasks=New-Object 'System.Collections.Generic.List[object]'
$active=New-Object 'System.Collections.Generic.List[object]'
$buildWatch=[Diagnostics.Stopwatch]::StartNew()
function Write-Report {
    $report=[ordered]@{profile=$profile;sample_rate=$sampleRate;diagnostic=(-not $FullCatalog);stage_only=[bool]$StageOnly;
        tracks=@($completed.ToArray());hardware_verified=$false;timing_gate_report=$TimingGateReport;elapsed_seconds=$buildWatch.Elapsed.TotalSeconds}
    [IO.File]::WriteAllText((Join-Path $OutputDirectory 'import-report.json'),(($report|ConvertTo-Json -Depth 12)+"`n"),$utf8)
}
function Finish-Track($task) {
    $track=$task.Track; $metadata=Get-Content -LiteralPath $task.MetadataPath -Raw|ConvertFrom-Json
    if ($metadata.profile -ne $profile -or $metadata.sample_rate -ne $sampleRate -or $metadata.channels -ne 2 -or
        $metadata.output_bits_per_sample -ne 8 -or $metadata.bits_per_sample -ne 3 -or $metadata.codebook_count -ne $task.Books -or
        $metadata.encoder_training -ne $track.encoder_options.training -or $metadata.encoder_beam_width -ne $track.encoder_options.beam_width) {
        throw "Encoded format mismatch: $($track.name)"
    }
    $actualHash=Get-Sha256 $task.AssetPath
    if ((Get-Sha256 $task.FlacPath) -ne $metadata.flac_sha256) { throw "Encoded source FLAC differs from the authenticated extraction: $($track.name)" }
    if ($actualHash -ne $metadata.asset_sha256 -or (Get-Item -LiteralPath $task.AssetPath).Length -ne $metadata.asset_bytes) {
        throw "Encoded hash or extent mismatch: $($track.name)"
    }
    if ($metadata.sample_count -ne $task.ExpectedFrames) { throw "Original duration was not preserved: $($track.name)" }
    if ($FullCatalog -and ($metadata.sample_count -ne $track.expected_frames -or $metadata.asset_bytes -ne $track.expected_asset_bytes)) {
        throw "Catalog capacity differs from actual encoding: $($track.name)"
    }
    $provenance=[ordered]@{attribution=$track.attribution;source=$track.source;source_verified_sha256=$task.SourceHash;
        conversion=[ordered]@{profile=$profile;diagnostic=(-not $FullCatalog);excerpt_start_seconds=0;
            excerpt_duration_seconds=$(if ($FullCatalog) {$null} else {$DiagnosticSeconds});
            decoded_source_frames=$task.SourceFrames;decoded_source_rate=[int]$track.source.sample_rate;output_frames=$metadata.sample_count;
            ffmpeg_version=$ffmpegVersion;encoder_sha256=$encoderHash;codebook_count=$metadata.codebook_count;
            training=$track.encoder_options.training;beam_width=$track.encoder_options.beam_width;asset_sha256=$actualHash;
            decoded_pcm_sha256=$metadata.decoded_pcm_sha256;reference_pcm_sha256=$metadata.reference_pcm_sha256;
            channel_processing='Preserve original left and right channels with one shared gain; no artificial delay'}}
    [IO.File]::WriteAllText($task.ProvenancePath,(($provenance|ConvertTo-Json -Depth 20)+"`n"),$utf8)
    $completed.Add([ordered]@{name=$track.name;asset=$track.asset;bytes=$metadata.asset_bytes;frames=$metadata.sample_count;asset_sha256=$actualHash})
    Write-Report
    Write-Host "Completed $($track.name): $($metadata.sample_count) frames, $($metadata.asset_bytes) bytes ($($completed.Count)/$($selected.Count))."
}
# Authenticate every original and preserve its complete decoded duration before encoding.
foreach ($track in $selected) {
    $sourcePath=Join-Path $SourceDirectory $track.source.filename
    $sourceHash=Get-Sha256 $sourcePath
    if ($sourceHash -ne $track.source.sha256) { throw "Original source hash mismatch: $($track.name)" }
    $probeText=& ffprobe -v error -select_streams a:0 -show_entries stream=sample_rate,channels -of json $sourcePath
    if ($LASTEXITCODE -ne 0) { throw "Source probe failed: $($track.name)" }
    $probe=($probeText -join "`n")|ConvertFrom-Json
    if ($probe.streams.Count -ne 1 -or $probe.streams[0].channels -ne 2 -or [int]$probe.streams[0].sample_rate -ne [int]$track.source.sample_rate) {
        throw "Unexpected original stereo shape: $($track.name)"
    }
    $flacPath=Join-Path $OutputDirectory ($track.name+'-source.flac')
    $ffmpegArgs=@('-v','error','-y','-i',$sourcePath,'-map','0:a:0','-vn')
    if (-not $FullCatalog) { $ffmpegArgs+=@('-t',$DiagnosticSeconds.ToString('R',[Globalization.CultureInfo]::InvariantCulture)) }
    $ffmpegArgs+=@('-sample_fmt','s16','-c:a','flac','-threads','1',$flacPath)
    & ffmpeg @ffmpegArgs
    if ($LASTEXITCODE -ne 0) { throw "FLAC preparation failed: $($track.name)" }
    $flacProbeText=& ffprobe -v error -select_streams a:0 -show_entries stream=sample_rate,channels,duration_ts,time_base -of json $flacPath
    if ($LASTEXITCODE -ne 0) { throw "FLAC frame-count probe failed: $($track.name)" }
    $flacProbe=($flacProbeText -join "`n")|ConvertFrom-Json; $stream=$flacProbe.streams[0]
    if ($stream.time_base -ne ('1/'+$track.source.sample_rate) -or $stream.channels -ne 2) { throw 'Unexpected lossless source time base or channels.' }
    $sourceFrames=[long]$stream.duration_ts
    if ($FullCatalog -and $sourceFrames -ne $track.expected_source_frames) { throw "Decoded original frame count differs: $($track.name)" }
    $expectedFrames=[long][math]::Ceiling($sourceFrames*[double]$sampleRate/[int]$track.source.sample_rate)
    $books=[int]$track.codebook_count
    if (-not $FullCatalog) { $books=[int][math]::Max(1,[math]::Ceiling($DiagnosticSeconds*$books/$track.source.duration_seconds)) }
    $task=[pscustomobject]@{Track=$track;SourceHash=$sourceHash;SourceFrames=$sourceFrames;ExpectedFrames=$expectedFrames;Books=$books;FlacPath=$flacPath;
        AssetPath=(Join-Path $OutputDirectory $track.asset);MetadataPath=(Join-Path $OutputDirectory $track.metadata);
        PreviewPath=(Join-Path $OutputDirectory ($track.name+'.wav'));ProvenancePath=(Join-Path $OutputDirectory ($track.name+'.provenance.json'));Process=$null}
    if ($Resume -and (Test-Path -LiteralPath $task.ProvenancePath) -and (Test-Path -LiteralPath $task.MetadataPath) -and (Test-Path -LiteralPath $task.AssetPath)) {
        $old=Get-Content -LiteralPath $task.ProvenancePath -Raw|ConvertFrom-Json
        if ($old.source_verified_sha256 -eq $sourceHash -and $old.conversion.encoder_sha256 -eq $encoderHash -and
            $old.conversion.profile -eq $profile -and $old.conversion.diagnostic -eq (-not $FullCatalog)) { Finish-Track $task;continue }
    }
    $tasks.Add($task)
}
$next=0
try {
    while ($next -lt $tasks.Count -or $active.Count) {
        while ($next -lt $tasks.Count -and $active.Count -lt $Jobs) {
            $task=$tasks[$next++];$track=$task.Track
            $arguments=@('--profile',$profile,'--codebooks',[string]$task.Books,'--training',$track.encoder_options.training,
                '--beam-width',[string]$track.encoder_options.beam_width,$task.FlacPath,$task.AssetPath,$task.PreviewPath)
            $argumentLine=($arguments|ForEach-Object {Quote-Argument ([string]$_)}) -join ' '
            $task.Process=Start-Process -FilePath $Encoder -ArgumentList $argumentLine -WindowStyle Hidden -PassThru -Wait:$false `
                -RedirectStandardOutput (Join-Path $OutputDirectory ($track.name+'.encoder.log')) `
                -RedirectStandardError (Join-Path $OutputDirectory ($track.name+'.encoder.stderr.log'))
            # Retain the native handle so Windows PowerShell can read ExitCode
            # after the child has exited (HasExited alone may otherwise lose it).
            $null=$task.Process.Handle
            $active.Add($task)
            Write-Host "Encoding $($track.name): $profile, $($task.Books) codebooks; $($active.Count)/$Jobs workers."
        }
        foreach ($task in @($active.ToArray())) {
            if ($task.Process.HasExited) {
                $task.Process.WaitForExit()
                if ($task.Process.ExitCode -ne 0) { throw "DLT5 encoding failed: $($task.Track.name), exit $($task.Process.ExitCode). See encoder logs." }
                Finish-Track $task
                [void]$active.Remove($task);$task.Process.Dispose()
            }
        }
        if ($active.Count) { Start-Sleep -Milliseconds 500 }
    }
} finally {
    foreach ($task in @($active.ToArray())) { if (-not $task.Process.HasExited) {$task.Process.Kill()};$task.Process.Dispose() }
}
Copy-Item -LiteralPath (Join-Path $projectRoot 'assets/music/LICENSE') -Destination (Join-Path $OutputDirectory 'MUSIC-LICENSE.txt')
Write-Report
Write-Host "Music assets staged at $OutputDirectory. Hardware verification remains false."
