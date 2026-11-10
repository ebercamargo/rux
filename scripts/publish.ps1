param(
    [switch]$SkipGitPush,
    [int]$WaitMinutes = 15
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

function Fail([string]$Message) {
    throw "RUX publish: $Message"
}
function Run([string]$Command, [string[]]$Arguments) {
    & $Command @Arguments
    if ($LASTEXITCODE -ne 0) { Fail "$Command $($Arguments -join ' ') failed (exit $LASTEXITCODE)" }
}
function Get-RegistryVersion([string]$PackageVersion) {
    $oldPreference = $ErrorActionPreference
    try {
        $ErrorActionPreference = 'Continue'
        $result = & npm view "rux-lang@$PackageVersion" version --registry=https://registry.npmjs.org/ 2>$null
        if ($LASTEXITCODE -eq 0 -and $result) { return $true }
        return $false
    } finally { $ErrorActionPreference = $oldPreference }
}

$root = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
Set-Location $root
if (-not (Get-Command git -ErrorAction SilentlyContinue)) { Fail 'Git is required.' }
if (-not (Get-Command npm -ErrorAction SilentlyContinue)) { Fail 'Node.js/npm is required.' }
if ((git branch --show-current).Trim() -ne 'main') { Fail 'Switch to main before publishing.' }
if (git status --porcelain) { Fail 'Working tree has uncommitted changes.' }

Write-Host 'Updating main...'
Run git @('fetch', 'origin', 'main', '--tags')
Run git @('pull', '--ff-only', 'origin', 'main')
if (git status --porcelain) { Fail 'Working tree changed after pull.' }

$version = (Get-Content (Join-Path $root 'VERSION') -Raw).Trim()
$packagePath = Join-Path $root 'npm/rux-lang'
$packageVersion = (Get-Content (Join-Path $packagePath 'package.json') -Raw | ConvertFrom-Json).version
$cargoVersion = [regex]::Match((Get-Content (Join-Path $root 'Cargo.toml') -Raw), '(?m)^version\s*=\s*"([^"]+)"').Groups[1].Value
if ($version -ne $packageVersion -or $version -ne $cargoVersion) {
    Fail "Version mismatch: VERSION=$version, npm=$packageVersion, Cargo=$cargoVersion. Merge the version update PR first."
}
if ($version -notmatch '^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$') { Fail "Invalid version: $version" }
$tag = "v$version"
Write-Host "Preparing $tag"

if (Get-RegistryVersion $version) {
    Fail "rux-lang@$version is already published. A new version is required; npm never permits overwriting."
}

Push-Location $packagePath
try {
    Run npm @('test')
    Run npm @('pack', '--dry-run', '--ignore-scripts')
} finally { Pop-Location }

if (Get-Command cargo -ErrorAction SilentlyContinue) {
    Run cargo @('test', '--all-targets')
} else {
    Write-Host 'Cargo not installed; GitHub Actions will compile and test Rust on Windows/Linux.'
}

$head = (git rev-parse HEAD).Trim()
$remoteTag = (& git ls-remote --tags origin "refs/tags/$tag" | Out-String).Trim()
if ($remoteTag) {
    $remoteSha = ($remoteTag -split '\s+')[0]
    $localTagSha = (git rev-parse "$tag^{}").Trim()
    if ($LASTEXITCODE -ne 0) { Fail "Cannot resolve existing tag $tag." }
    if ($localTagSha -ne $head) {
        # A release tag may legitimately predate later documentation or automation commits.
        # Verify that its exact package, compiler and version files match the current release.
        foreach ($releaseFile in @('VERSION', 'Cargo.toml', 'npm/rux-lang/package.json', 'npm/rux-lang/scripts/install.cjs', 'npm/rux-lang/bin/rux.cjs')) {
            $taggedContent = git show "${tag}:$releaseFile"
            if ($LASTEXITCODE -ne 0) { Fail "Cannot read $releaseFile from existing tag $tag." }
            $currentContent = Get-Content (Join-Path $root $releaseFile) -Raw
            if (($taggedContent -join "`n").TrimEnd() -ne $currentContent.TrimEnd()) {
                Fail "$tag already exists with different release file $releaseFile. Create a new version; never move the tag."
            }
        }
        Write-Host "$tag predates current main, but release files match. Reusing immutable tag."
    }
    Write-Host "$tag already exists on origin."
} else {
    if ($SkipGitPush) { Fail "Tag $tag does not exist; cannot continue with -SkipGitPush." }
    if (git tag --list $tag) {
        $tagSha = (git rev-parse "$tag^{}").Trim()
        if ($tagSha -ne $head) { Fail "Local tag $tag points to a different commit." }
    } else {
        Run git @('tag', $tag)
    }
    Run git @('push', 'origin', $tag)
    Write-Host "Tag $tag pushed. GitHub Actions will build the binaries."
}

$releaseUrl = "https://api.github.com/repos/ebercamargo/rux/releases/tags/$tag"
$requiredAssets = @(
    'rux-windows-x86_64.zip',
    'rux-windows-x86_64.zip.sha256',
    'rux-linux-x86_64.tar.gz',
    'rux-linux-x86_64.tar.gz.sha256'
)
$headers = @{ 'User-Agent' = 'rux-release-script'; 'Accept' = 'application/vnd.github+json' }
$deadline = (Get-Date).AddMinutes($WaitMinutes)
Write-Host "Waiting for release assets ($WaitMinutes minutes maximum)..."
$ready = $false
while ((Get-Date) -lt $deadline) {
    try {
        $release = Invoke-RestMethod -Uri $releaseUrl -Headers $headers -TimeoutSec 20
        $names = @($release.assets | ForEach-Object { $_.name })
        $missing = @($requiredAssets | Where-Object { $_ -notin $names })
        if ($missing.Count -eq 0) { $ready = $true; break }
        Write-Host "Waiting for: $($missing -join ', ')"
    } catch {
        Write-Host 'Release is not available yet...'
    }
    Start-Sleep -Seconds 15
}
if (-not $ready) {
    Fail "Release assets were not ready. Check https://github.com/ebercamargo/rux/actions/workflows/release.yml and rerun this script later."
}

# Recheck before publishing to prevent accidental duplicate attempts.
if (Get-RegistryVersion $version) { Fail "rux-lang@$version was published while waiting. Aborting duplicate publication." }

Write-Host "Publishing rux-lang@$version to npm. Browser authentication may be required."
Push-Location $packagePath
try {
    Run npm @('publish', '--access', 'public', '--tag', 'alpha', '--auth-type=web')
} finally { Pop-Location }
Write-Host "Published rux-lang@$version. Verify with: npm view rux-lang dist-tags"
