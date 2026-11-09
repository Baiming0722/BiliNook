$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Push-Location -LiteralPath $root
try {
    & (Join-Path $PSScriptRoot 'cargo.ps1') build --locked --release -p bilinook
    if ($LASTEXITCODE -ne 0) { throw 'Release 建置失敗' }
    New-Item -ItemType Directory -Path (Join-Path $root 'dist') -Force | Out-Null
    $binary = Join-Path $root 'dist/BiliNook.exe'
    Copy-Item -LiteralPath (Join-Path $root 'target/release/BiliNook.exe') -Destination $binary
    $artifact = Get-Item -LiteralPath $binary
    [ordered]@{
        productName = $artifact.VersionInfo.ProductName
        fileName = $artifact.Name
        version = $artifact.VersionInfo.ProductVersion
        sizeBytes = $artifact.Length
        sha256 = (Get-FileHash -LiteralPath $binary -Algorithm SHA256).Hash
        builtAt = [DateTimeOffset]::Now.ToString('o')
    } | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $root 'dist/build-info.json') -Encoding utf8
    Write-Output $binary
} finally { Pop-Location }
