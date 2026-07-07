param(
    [string]$OutputPath = "",
    [string]$PosterPath = "",
    [int]$Fps = 30,
    [int]$DurationSeconds = 18,
    [switch]$KeepFrames
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$repoRoot = Resolve-Path (Join-Path $PSScriptRoot "..\..")
if (-not $OutputPath) {
    $OutputPath = Join-Path $repoRoot "docs\assets\gridbash-openvid-demo.mp4"
}
if (-not $PosterPath) {
    $PosterPath = Join-Path $repoRoot "docs\assets\gridbash-openvid-demo-poster.png"
}

$node = (Get-Command node -ErrorAction Stop).Source
$script = Join-Path $PSScriptRoot "capture-openvid-demo.mjs"

$args = @(
    $script,
    "--output", $OutputPath,
    "--poster", $PosterPath,
    "--fps", $Fps,
    "--duration", $DurationSeconds
)

if ($KeepFrames) {
    $args += "--keep-frames"
}

& $node @args
exit $LASTEXITCODE
