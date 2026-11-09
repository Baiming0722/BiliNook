$CargoArgs = $args
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$env:CARGO_HOME = Join-Path $projectRoot '.cargo-home'
$env:CARGO_NET_RETRY = '1'
Push-Location -LiteralPath $projectRoot
try {
    & cargo @CargoArgs
    exit $LASTEXITCODE
} finally {
    Pop-Location
}

