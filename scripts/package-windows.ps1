[CmdletBinding()]
param(
    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

function Get-Sha256([string]$LiteralPath) {
    $stream = [System.IO.File]::OpenRead($LiteralPath)
    $algorithm = [System.Security.Cryptography.SHA256]::Create()
    try {
        return ([System.BitConverter]::ToString($algorithm.ComputeHash($stream))).Replace("-", "").ToLowerInvariant()
    }
    finally {
        $algorithm.Dispose()
        $stream.Dispose()
    }
}

$repositoryRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot ".."))
$distRoot = [System.IO.Path]::GetFullPath((Join-Path $repositoryRoot "dist"))
if ([System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture -ne [System.Runtime.InteropServices.Architecture]::X64) {
    throw "OpenManic Windows release packages must be built on x86-64 Windows."
}

Push-Location $repositoryRoot
try {
    $metadataJson = & cargo metadata --no-deps --locked --format-version 1
    if ($LASTEXITCODE -ne 0) {
        throw "cargo metadata failed with exit code $LASTEXITCODE"
    }
    $metadata = $metadataJson | ConvertFrom-Json
    $applicationPackage = @($metadata.packages | Where-Object { $_.name -eq "openmanic" })
    if ($applicationPackage.Count -ne 1) {
        throw "Expected exactly one openmanic package in cargo metadata."
    }
    $version = [string]$applicationPackage[0].version

    if (-not $SkipBuild) {
        & cargo build -p openmanic --release --no-default-features --features "renderer-wgpu,platform-windows" --locked
        if ($LASTEXITCODE -ne 0) {
            throw "The OpenManic release build failed with exit code $LASTEXITCODE"
        }
    }

    $executable = Join-Path $repositoryRoot "target/release/openmanic.exe"
    if (-not (Test-Path -LiteralPath $executable -PathType Leaf)) {
        throw "The release executable is missing. Build it before packaging."
    }

    $artifactBase = "OpenManic-v$version-windows-x86_64"
    $stageRoot = [System.IO.Path]::GetFullPath((Join-Path $distRoot $artifactBase))
    $zipPath = [System.IO.Path]::GetFullPath((Join-Path $distRoot "$artifactBase.zip"))
    $checksumPath = "$zipPath.sha256"
    $expectedPrefix = $distRoot.TrimEnd([System.IO.Path]::DirectorySeparatorChar) + [System.IO.Path]::DirectorySeparatorChar
    if (-not $stageRoot.StartsWith($expectedPrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "Refusing to stage outside the repository dist directory."
    }

    New-Item -ItemType Directory -Path $distRoot -Force | Out-Null
    if (Test-Path -LiteralPath $stageRoot) {
        Remove-Item -LiteralPath $stageRoot -Recurse -Force
    }
    foreach ($output in @($zipPath, $checksumPath)) {
        if (Test-Path -LiteralPath $output) {
            Remove-Item -LiteralPath $output -Force
        }
    }
    New-Item -ItemType Directory -Path $stageRoot | Out-Null

    Copy-Item -LiteralPath $executable -Destination (Join-Path $stageRoot "OpenManic.exe")
    Copy-Item -LiteralPath (Join-Path $repositoryRoot "packaging/windows/README.txt") -Destination (Join-Path $stageRoot "README.txt")

    $revision = (& git rev-parse HEAD).Trim()
    if ($LASTEXITCODE -ne 0) {
        throw "Could not determine the Git revision for the release manifest."
    }
    $trackedChanges = @(& git status --porcelain --untracked-files=no)
    if ($LASTEXITCODE -ne 0) {
        throw "Could not determine whether the release source tree is clean."
    }
    $sourceState = if ($trackedChanges.Count -eq 0) { "clean" } else { "dirty" }
    $executableHash = Get-Sha256 $executable
    @(
        "OpenManic portable Windows release"
        "Version: $version"
        "Git revision: $revision"
        "Source tree: $sourceState"
        "Architecture: x86_64"
        "Renderer: WGPU"
        "Executable SHA-256: $executableHash"
    ) | Set-Content -LiteralPath (Join-Path $stageRoot "BUILD-INFO.txt") -Encoding UTF8

    Compress-Archive -LiteralPath $stageRoot -DestinationPath $zipPath -CompressionLevel Optimal
    $zipHash = Get-Sha256 $zipPath
    "$zipHash  $([System.IO.Path]::GetFileName($zipPath))" | Set-Content -LiteralPath $checksumPath -Encoding ASCII -NoNewline

    Write-Host "Created $zipPath"
    Write-Host "Created $checksumPath"
}
finally {
    Pop-Location
}
