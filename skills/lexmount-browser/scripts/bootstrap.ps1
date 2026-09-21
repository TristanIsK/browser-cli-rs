$ErrorActionPreference = "Stop"

function Invoke-Tls12Download {
  param(
    [Parameter(Mandatory = $true)][string]$Uri,
    [Parameter(Mandatory = $true)][string]$OutFile
  )
  try {
    Invoke-WebRequest -UseBasicParsing -Uri $Uri -OutFile $OutFile
  } catch {
    throw "Failed to download $Uri with TLS 1.2 enabled: $($_.Exception.Message)"
  }
}

# Release pins are reviewed with this package. Reject environment source/version/path overrides.
foreach ($key in @('LEXMOUNT_BROWSER_CLI_VERSION', 'LEXMOUNT_BROWSER_CLI_DOWNLOAD_BASE_URL', 'LEXMOUNT_BROWSER_CLI_INSTALL_DIR')) {
  if ([Environment]::GetEnvironmentVariable($key)) { throw "Release overrides are disabled; unset $key." }
}
$version = if ($env:LEXMOUNT_BROWSER_CLI_VERSION) { $env:LEXMOUNT_BROWSER_CLI_VERSION } else { "1.2.3" }
$downloadBaseUrl = if ($env:LEXMOUNT_BROWSER_CLI_DOWNLOAD_BASE_URL) { $env:LEXMOUNT_BROWSER_CLI_DOWNLOAD_BASE_URL.TrimEnd('/') } else { "https://cli-bin-1377899528.cos.ap-nanjing.myqcloud.com/releases/browser-cli" }
$architecture = if ($env:PROCESSOR_ARCHITEW6432) { $env:PROCESSOR_ARCHITEW6432 } else { $env:PROCESSOR_ARCHITECTURE }
if ($architecture -ne "AMD64") { throw "Only Windows x64 is supported" }
$asset = "browser-cli-v$version-x86_64-pc-windows-msvc.exe"
$repo = "$downloadBaseUrl/v$version"
$tmp = Join-Path ([IO.Path]::GetTempPath()) ([Guid]::NewGuid().ToString())
New-Item -ItemType Directory -Path $tmp | Out-Null
try {
  $previousSecurityProtocol = [Net.ServicePointManager]::SecurityProtocol
  try {
    [Net.ServicePointManager]::SecurityProtocol = $previousSecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
  } catch {
    throw "Failed to enable TLS 1.2 for COS downloads: $($_.Exception.Message)"
  }
  try {
    Invoke-Tls12Download "$repo/$asset" (Join-Path $tmp $asset)
  } finally {
    [Net.ServicePointManager]::SecurityProtocol = $previousSecurityProtocol
  }
  $expected = "60c8fd5c9d501de5224363e08fa55022fa3af68fc3fcfef10be7609ccb4d7bae"
  $actual = (Get-FileHash (Join-Path $tmp $asset) -Algorithm SHA256).Hash.ToLowerInvariant()
  if ($expected -ne $actual) { throw "SHA-256 mismatch for $asset" }
  $skillDir = Split-Path -Parent $PSScriptRoot
  $installDir = if ($env:LEXMOUNT_BROWSER_CLI_INSTALL_DIR) { $env:LEXMOUNT_BROWSER_CLI_INSTALL_DIR } else { Join-Path $skillDir "bin" }
  New-Item -ItemType Directory -Path $installDir -Force | Out-Null
  foreach ($path in @($installDir, (Join-Path $installDir 'browser-cli.exe'))) {
    if ((Test-Path -LiteralPath $path) -and ((Get-Item -LiteralPath $path -Force).Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw "Refusing reparse-point installation target" }
  }
  & (Join-Path $tmp $asset) version
  if ($LASTEXITCODE -ne 0) { throw "Downloaded browser-cli failed verification (exit $LASTEXITCODE)" }
  Copy-Item (Join-Path $tmp $asset) (Join-Path $installDir "browser-cli.exe") -Force
  & (Join-Path $installDir "browser-cli.exe") version
  if ($LASTEXITCODE -ne 0) { throw "Installed browser-cli failed verification (exit $LASTEXITCODE)" }
  Write-Output "Installed browser-cli to $installDir\browser-cli.exe"
} finally { Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue }
