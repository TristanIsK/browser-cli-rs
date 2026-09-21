# Dependency-free release gate regression tests; compatible with PowerShell 5.1.
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$validator = Join-Path $PSScriptRoot 'verify-release-version.ps1'
$fixture = Join-Path ([IO.Path]::GetTempPath()) ('browser-cli-version-' + [Guid]::NewGuid().ToString('N'))
$resolvedFixture = [IO.Path]::GetFullPath($fixture)
$tempRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd([IO.Path]::DirectorySeparatorChar) + [IO.Path]::DirectorySeparatorChar
if (-not $resolvedFixture.StartsWith($tempRoot, [StringComparison]::OrdinalIgnoreCase)) { throw 'Fixture is outside temporary directory' }
New-Item -ItemType Directory -Path (Join-Path $fixture 'skills/lexmount-browser/scripts') -Force | Out-Null
$cases = [Collections.Generic.List[string]]::new()
$utf8 = [Text.UTF8Encoding]::new($false)

function Write-Fixture([string]$Path, [string]$Text) {
  [IO.File]::WriteAllText((Join-Path $fixture $Path), $Text, $utf8)
}
function Reset-Fixture([string]$Version = '1.2.3') {
  Write-Fixture 'Cargo.toml' "[package]`nname = `"lexmount-browser`"`nversion = `"$Version`"`n[dependencies]`nother = `"9.8.7`"`n"
  Write-Fixture 'Cargo.lock' "version = 4`n[[package]]`nname = `"dependency`"`nversion = `"9.8.7`"`n[[package]]`nname = `"lexmount-browser`"`nversion = `"$Version`"`n"
  Write-Fixture 'skills/lexmount-browser/scripts/bootstrap.ps1' ('$version = if ($env:LEXMOUNT_BROWSER_CLI_VERSION) { $env:LEXMOUNT_BROWSER_CLI_VERSION } else { "' + $Version + '" }' + "`n")
  Write-Fixture 'skills/lexmount-browser/scripts/bootstrap.sh' ('version="${LEXMOUNT_BROWSER_CLI_VERSION:-' + $Version + '}"' + "`n")
}
function Pass([string]$Name, [scriptblock]$Action) {
  & $Action
  $cases.Add($Name)
  Write-Host "PASS $Name"
}
function Reject([string]$Name, [scriptblock]$Action, [string]$Expected) {
  $errorText = $null
  try { & $Action | Out-Null } catch { $errorText = $_.Exception.Message }
  if (-not $errorText -or $errorText -notlike "*$Expected*") { throw "$Name did not reject with '$Expected': $errorText" }
  $cases.Add($Name)
  Write-Host "PASS $Name"
}
try {
  Reset-Fixture
  Pass 'matching source and tag' {
    $result = & $validator -RepositoryRoot $fixture -ReleaseTag 'v1.2.3'
    if ($result.Version -cne '1.2.3' -or $result.VerifiedFiles -ne 4) { throw 'Unexpected validation result' }
  }
  Pass 'source-only PR check' { & $validator -RepositoryRoot $fixture | Out-Null }
  foreach ($tag in @('v1.2.2', '1.2.3', 'V1.2.3', 'v1.2.3-extra', '')) {
    Reject "wrong tag [$tag]" { & $validator -RepositoryRoot $fixture -ReleaseTag $tag } 'Release tag'
  }
  Reset-Fixture '1.2.1'
  Reject 'published 1.2.2 incident: every source version still 1.2.1' { & $validator -RepositoryRoot $fixture -ReleaseTag 'v1.2.2' } 'Release tag'
  Reset-Fixture
  Write-Fixture 'Cargo.lock' "[[package]]`nname = `"lexmount-browser`"`nversion = `"1.2.1`"`n"
  Reject 'stale lockfile' { & $validator -RepositoryRoot $fixture } 'Cargo.lock version'
  Reset-Fixture
  Write-Fixture 'skills/lexmount-browser/scripts/bootstrap.ps1' '$version = if ($env:LEXMOUNT_BROWSER_CLI_VERSION) { $env:LEXMOUNT_BROWSER_CLI_VERSION } else { "1.2.1" }'
  Reject 'stale Windows bootstrap' { & $validator -RepositoryRoot $fixture } 'bootstrap.ps1 version'
  Reset-Fixture
  Write-Fixture 'skills/lexmount-browser/scripts/bootstrap.sh' 'version="${LEXMOUNT_BROWSER_CLI_VERSION:-1.2.1}"'
  Reject 'stale Mac bootstrap' { & $validator -RepositoryRoot $fixture } 'bootstrap.sh version'
  Reset-Fixture
  Write-Fixture 'Cargo.lock' "[[package]]`nname = `"some-dependency`"`nversion = `"1.2.3`"`n"
  Reject 'dependency version cannot stand in for package' { & $validator -RepositoryRoot $fixture } 'exactly one lexmount-browser'
  Reset-Fixture
  $lock = [IO.File]::ReadAllText((Join-Path $fixture 'Cargo.lock'))
  Write-Fixture 'Cargo.lock' ($lock + "[[package]]`nname = `"lexmount-browser`"`nversion = `"1.2.3`"`n")
  Reject 'duplicate package' { & $validator -RepositoryRoot $fixture } 'exactly one lexmount-browser'
  Reset-Fixture
  Write-Fixture 'skills/lexmount-browser/scripts/bootstrap.sh' '# version="${LEXMOUNT_BROWSER_CLI_VERSION:-1.2.3}"'
  Reject 'comment is not a bootstrap version' { & $validator -RepositoryRoot $fixture } 'shell bootstrap default'
  Reset-Fixture
  $bootstrap = [IO.File]::ReadAllText((Join-Path $fixture 'skills/lexmount-browser/scripts/bootstrap.sh'))
  Write-Fixture 'skills/lexmount-browser/scripts/bootstrap.sh' ($bootstrap + $bootstrap)
  Reject 'duplicate bootstrap declarations' { & $validator -RepositoryRoot $fixture } 'shell bootstrap default'
  Reset-Fixture '1.2.3-rc.1'
  Pass 'matching prerelease' { & $validator -RepositoryRoot $fixture -ReleaseTag 'v1.2.3-rc.1' | Out-Null }
  Reset-Fixture
  foreach ($path in @('Cargo.toml', 'Cargo.lock', 'skills/lexmount-browser/scripts/bootstrap.ps1', 'skills/lexmount-browser/scripts/bootstrap.sh')) {
    Write-Fixture $path ([IO.File]::ReadAllText((Join-Path $fixture $path)).Replace("`n", "`r`n"))
  }
  Pass 'Windows line endings' { & $validator -RepositoryRoot $fixture -ReleaseTag 'v1.2.3' | Out-Null }
  Pass 'actual repository source' { & $validator | Out-Null }
  Pass 'standalone -File entry point' {
    $currentVersion = (& $validator).Version
    $engine = (Get-Process -Id $PID).Path
    & $engine -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $validator -ReleaseTag "v$currentVersion" | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "Standalone release gate exited $LASTEXITCODE" }
  }
  [pscustomobject]@{ Passed = $cases.Count; Cases = @($cases) } | ConvertTo-Json -Depth 3
} finally {
  # Only this invocation's prevalidated GUID fixture, never the temp root.
  Remove-Item -LiteralPath $resolvedFixture -Recurse -Force
}
