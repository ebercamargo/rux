param(
    [string]$Version = "latest",
    [string]$InstallDir = "$env:LOCALAPPDATA\RUX\bin"
)
$ErrorActionPreference = "Stop"
$repo = "ebercamargo/rux"
if (-not [Environment]::Is64BitOperatingSystem) { throw "RUX requires 64-bit Windows." }
$release = if ($Version -eq "latest") { "latest" } else { "tags/$Version" }
$api = "https://api.github.com/repos/$repo/releases/$release"
$headers = @{ "User-Agent" = "rux-installer"; "Accept" = "application/vnd.github+json" }
try { $data = Invoke-RestMethod -Uri $api -Headers $headers } catch {
    throw "No published RUX release found. The installer will work after the first GitHub Release is published. $($_.Exception.Message)"
}
$assetName = "rux-windows-x86_64.zip"
$asset = $data.assets | Where-Object { $_.name -eq $assetName } | Select-Object -First 1
if (-not $asset) { throw "Release $($data.tag_name) has no $assetName artifact." }
$temp = Join-Path ([IO.Path]::GetTempPath()) ("rux-install-" + [Guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Force -Path $temp | Out-Null
try {
    $archive = Join-Path $temp $assetName
    Invoke-WebRequest -Uri $asset.browser_download_url -OutFile $archive
    $expected = $data.assets | Where-Object { $_.name -eq "$assetName.sha256" } | Select-Object -First 1
    if (-not $expected) { throw "Missing SHA256 checksum for $assetName." }
    $checksumFile = Join-Path $temp "$assetName.sha256"
    Invoke-WebRequest -Uri $expected.browser_download_url -OutFile $checksumFile
    $hash = (Get-FileHash $archive -Algorithm SHA256).Hash.ToLowerInvariant()
    $expectedHash = ((Get-Content $checksumFile -Raw).Trim() -split '\s+')[0].ToLowerInvariant()
    if ($hash -ne $expectedHash) { throw "SHA256 mismatch; installation aborted." }
    Expand-Archive -Path $archive -DestinationPath $temp -Force
    $binary = Join-Path $temp "rux.exe"
    if (-not (Test-Path $binary)) { throw "Archive does not contain rux.exe." }
    New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
    Copy-Item $binary (Join-Path $InstallDir "rux.exe") -Force
    $userPath = [Environment]::GetEnvironmentVariable("Path", "User")
    $entries = @($userPath -split ';' | Where-Object { $_ })
    if ($InstallDir -notin $entries) {
        [Environment]::SetEnvironmentVariable("Path", (($entries + $InstallDir) -join ';'), "User")
    }
    if ($InstallDir -notin @($env:Path -split ';')) { $env:Path += ";$InstallDir" }
    & (Join-Path $InstallDir "rux.exe") --version
    Write-Host "RUX installed. Open a new terminal to use 'rux'."
} finally {
    Remove-Item -Recurse -Force $temp -ErrorAction SilentlyContinue
}
