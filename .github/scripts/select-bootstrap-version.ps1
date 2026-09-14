# CI-only selection. A Git tag alone does not prove that COS upload completed.
[CmdletBinding()]
param(
  [AllowEmptyCollection()][string[]]$Tags,
  [string]$DownloadBaseUrl = $env:LEXMOUNT_BROWSER_CLI_DOWNLOAD_BASE_URL
)

$ErrorActionPreference = "Stop"
if (-not $DownloadBaseUrl) {
  $DownloadBaseUrl = "https://cli-bin-1377899528.cos.ap-nanjing.myqcloud.com/releases/browser-cli"
}
$DownloadBaseUrl = $DownloadBaseUrl.TrimEnd('/')
if (-not $PSBoundParameters.ContainsKey('Tags')) {
  $repositoryRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
  $Tags = @(git -C $repositoryRoot tag --list "v*")
  if ($LASTEXITCODE -ne 0) { throw "Could not list release tags" }
}
$candidates = @($Tags | Where-Object { $_ -cmatch '^v[0-9]+\.[0-9]+\.[0-9]+$' } |
  Sort-Object { [version]$_.Substring(1) } -Descending -Unique)

function Invoke-ReleaseProbe {
  param([string]$Uri, [string]$Method)
  try {
    $response = Invoke-WebRequest -UseBasicParsing -Uri $Uri -Method $Method -TimeoutSec 15
    if ([int]$response.StatusCode -ne 200) {
      throw "Unexpected HTTP status $($response.StatusCode) for $Uri"
    }
    return $response
  } catch {
    # Only a missing object means this candidate is not ready. Do not conceal
    # outages, authentication failures, TLS errors or timeouts by downgrading.
    if ($_.Exception.Response -and [int]$_.Exception.Response.StatusCode -eq 404) {
      return $null
    }
    throw
  }
}

$previousSecurityProtocol = [Net.ServicePointManager]::SecurityProtocol
try {
  [Net.ServicePointManager]::SecurityProtocol = $previousSecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
  foreach ($tag in $candidates) {
    $version = $tag.Substring(1)
    $asset = "browser-cli-v$version-x86_64-pc-windows-msvc.exe"
    $releaseUrl = "$DownloadBaseUrl/$tag"
    # The release uploader publishes SHA256SUMS last, after the binaries.
    $manifest = Invoke-ReleaseProbe "$releaseUrl/SHA256SUMS" "GET"
    if ($null -eq $manifest) {
      Write-Warning "Skipping ${tag}: checksum manifest is not published."
      continue
    }
    $content = if ($manifest.Content -is [byte[]]) {
      [Text.Encoding]::UTF8.GetString($manifest.Content)
    } else { [string]$manifest.Content }
    $lines = @($content -split '\r?\n' | Where-Object { $_ -notmatch '^\s*$' })
    if ($lines.Count -eq 0 -or @($lines | Where-Object { $_ -notmatch '^[a-fA-F0-9]{64}\s+\*?\S+$' }).Count -gt 0) {
      throw "Invalid checksum manifest for $tag"
    }
    $assetPattern = [regex]::Escape($asset)
    $entries = @($lines | Where-Object { $_ -match "\s+\*?$assetPattern`$" })
    if ($entries.Count -eq 0) {
      Write-Warning "Skipping ${tag}: no Windows checksum entry is published."
      continue
    }
    if ($entries.Count -ne 1 -or $entries[0] -notmatch "^[a-fA-F0-9]{64}\s+\*?$assetPattern`$") {
      throw "Invalid or duplicate checksum entry for $asset"
    }
    if ($null -eq (Invoke-ReleaseProbe "$releaseUrl/$asset" "HEAD")) {
      Write-Warning "Skipping ${tag}: Windows binary is not published."
      continue
    }
    # Select once. The caller must still run the real bootstrap and its hash
    # check; an installation failure must not retry an older version.
    return $version
  }
  throw "No published Windows binary with a checksum found among $($candidates.Count) release tags."
} finally {
  [Net.ServicePointManager]::SecurityProtocol = $previousSecurityProtocol
}
