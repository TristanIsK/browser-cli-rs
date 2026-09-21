$ErrorActionPreference = 'Stop'
$root = Join-Path ([IO.Path]::GetTempPath()) ([Guid]::NewGuid().ToString())
$script:networkCalled = $false
function Invoke-WebRequest {
  param($Uri, $OutFile, [switch]$UseBasicParsing)
  $script:networkCalled = $true
  [IO.File]::WriteAllText($OutFile, 'untrusted executable payload')
}
try {
  New-Item -ItemType Directory -Path "$root/scripts", "$root/bin" | Out-Null
  Copy-Item "$PSScriptRoot/../skills/lexmount-browser/scripts/bootstrap.ps1" "$root/scripts/bootstrap.ps1"
  [IO.File]::WriteAllText("$root/bin/browser-cli.exe", 'previous binary')
  foreach ($key in @('LEXMOUNT_BROWSER_CLI_VERSION', 'LEXMOUNT_BROWSER_CLI_DOWNLOAD_BASE_URL', 'LEXMOUNT_BROWSER_CLI_INSTALL_DIR')) {
    $old = [Environment]::GetEnvironmentVariable($key)
    try {
      [Environment]::SetEnvironmentVariable($key, 'untrusted-override')
      $failed = $false
      try { & "$root/scripts/bootstrap.ps1" } catch {
        if ($_.Exception.Message -notmatch 'overrides are disabled') { throw }
        $failed = $true
      }
      if (-not $failed -or $script:networkCalled) { throw 'Override reached network/install' }
    } finally { [Environment]::SetEnvironmentVariable($key, $old) }
  }
  $failed = $false
  try { & "$root/scripts/bootstrap.ps1" } catch {
    if ($_.Exception.Message -notmatch 'SHA-256 mismatch') { throw }
    $failed = $true
  }
  if (-not $failed -or -not $script:networkCalled) { throw 'Invalid payload was not rejected' }
  if ([IO.File]::ReadAllText("$root/bin/browser-cli.exe") -cne 'previous binary') { throw 'Previous install changed' }
  Write-Output 'PowerShell override and checksum failure checks passed'
} finally { Remove-Item -LiteralPath $root -Recurse -Force }
