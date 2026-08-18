param(
    [Parameter(Mandatory = $true)]
    [string] $Root,

    [string] $Repo = "xoo0524tw/Tenacity-Launcher",

    [string] $AssetName = "Tenacity.jar"
)

$ErrorActionPreference = "Stop"

$normalizedRoot = $Root.Trim().Trim('"')
$rootPath = [System.IO.Path]::GetFullPath($normalizedRoot)
$savePath = Join-Path $rootPath "save"
$jarPath = Join-Path $rootPath $AssetName
$versionFile = Join-Path $savePath "Tenacity.version"
$tempPath = Join-Path $savePath "$AssetName.download"

New-Item -ItemType Directory -Force -Path $savePath | Out-Null

[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

$headers = @{
    "User-Agent" = "Tenacity-Launcher"
    "Accept" = "application/vnd.github+json"
}

Write-Host "[Updater] Checking latest Tenacity release..."

$release = Invoke-RestMethod `
    -Uri "https://api.github.com/repos/$Repo/releases/latest" `
    -Headers $headers `
    -UseBasicParsing

$latestTag = [string] $release.tag_name
if ([string]::IsNullOrWhiteSpace($latestTag)) {
    throw "The latest GitHub release has no tag name."
}

$asset = $release.assets |
    Where-Object { $_.name -eq $AssetName } |
    Select-Object -First 1

if (-not $asset) {
    $asset = $release.assets |
        Where-Object { $_.name -like "*.jar" -and $_.name -like "*Tenacity*" } |
        Select-Object -First 1
}

if (-not $asset) {
    throw "Release $latestTag does not contain $AssetName."
}

$localTag = ""
if (Test-Path -LiteralPath $versionFile) {
    $localTag = (Get-Content -Raw -LiteralPath $versionFile).Trim()
}

if ((Test-Path -LiteralPath $jarPath) -and $localTag -eq $latestTag) {
    Write-Host "[Updater] Tenacity.jar is up to date ($latestTag)."
    exit 0
}

if (Test-Path -LiteralPath $jarPath) {
    Write-Host "[Updater] Updating Tenacity.jar: $localTag -> $latestTag"
} else {
    Write-Host "[Updater] Downloading Tenacity.jar ($latestTag)"
}

if (Test-Path -LiteralPath $tempPath) {
    Remove-Item -Force -LiteralPath $tempPath
}

Invoke-WebRequest `
    -Uri $asset.browser_download_url `
    -OutFile $tempPath `
    -Headers @{ "User-Agent" = "Tenacity-Launcher" } `
    -UseBasicParsing

$downloaded = Get-Item -LiteralPath $tempPath
if ($downloaded.Length -lt 1048576) {
    throw "Downloaded Tenacity.jar is unexpectedly small."
}

Move-Item -Force -LiteralPath $tempPath -Destination $jarPath
Set-Content -LiteralPath $versionFile -Value $latestTag -Encoding ASCII

Write-Host "[Updater] Tenacity.jar is ready ($latestTag)."
