# Source-only release gate. Never runs bootstrap scripts or changes versions.
[CmdletBinding()]
param(
  [string]$RepositoryRoot,
  [AllowEmptyString()][string]$ReleaseTag
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if (-not $PSBoundParameters.ContainsKey('RepositoryRoot')) {
  $RepositoryRoot = Join-Path $PSScriptRoot '../..'
}

function Read-OneMatch([string]$Text, [string]$Pattern, [string]$Label) {
  $found = [regex]::Matches($Text, $Pattern)
  if ($found.Count -ne 1) { throw "Expected exactly one $Label; found $($found.Count)" }
  return $found[0].Groups[1].Value
}

# These deliberately accept the repository's literal version declarations only.
# Fail closed if the manifest/bootstrap layout changes; never guess a version
# from a comment, dependency, environment override or a different package.
$manifest = Get-Content -LiteralPath (Join-Path $RepositoryRoot 'Cargo.toml') -Raw
$package = Read-OneMatch $manifest '(?ms)^\[package\][ \t]*\r?\n(.*?)(?=^\[|\z)' 'Cargo.toml [package] section'
$name = Read-OneMatch $package '(?m)^name[ \t]*=[ \t]*"([^"]+)"[ \t]*\r?$' 'package name'
if ($name -cne 'lexmount-browser') { throw "Unexpected package name: $name" }
$version = Read-OneMatch $package '(?m)^version[ \t]*=[ \t]*"([^"]+)"[ \t]*\r?$' 'package version'
$semver = '(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(?:-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?'
if ($version -cnotmatch "^$semver$") { throw "Invalid literal package version: $version" }

$lock = Get-Content -LiteralPath (Join-Path $RepositoryRoot 'Cargo.lock') -Raw
$entries = @([regex]::Matches($lock, '(?ms)^\[\[package\]\][ \t]*\r?\n(.*?)(?=^\[|\z)') |
  Where-Object { $_.Groups[1].Value -match '(?m)^name[ \t]*=[ \t]*"lexmount-browser"[ \t]*\r?$' })
if ($entries.Count -ne 1) { throw 'Expected exactly one lexmount-browser entry in Cargo.lock' }
$lockVersion = Read-OneMatch $entries[0].Groups[1].Value '(?m)^version[ \t]*=[ \t]*"([^"]+)"[ \t]*\r?$' 'Cargo.lock package version'
$psBootstrap = Get-Content -LiteralPath (Join-Path $RepositoryRoot 'skills/lexmount-browser/scripts/bootstrap.ps1') -Raw
$shBootstrap = Get-Content -LiteralPath (Join-Path $RepositoryRoot 'skills/lexmount-browser/scripts/bootstrap.sh') -Raw
$psVersion = Read-OneMatch $psBootstrap '(?m)^\$version = if \(\$env:LEXMOUNT_BROWSER_CLI_VERSION\) \{ \$env:LEXMOUNT_BROWSER_CLI_VERSION \} else \{ "([^"]+)" \}[ \t]*\r?$' 'PowerShell bootstrap default'
$shVersion = Read-OneMatch $shBootstrap '(?m)^version="\$\{LEXMOUNT_BROWSER_CLI_VERSION:-([^}]+)\}"[ \t]*\r?$' 'shell bootstrap default'
foreach ($entry in @(
  @{ Label = 'Cargo.lock'; Value = $lockVersion },
  @{ Label = 'bootstrap.ps1'; Value = $psVersion },
  @{ Label = 'bootstrap.sh'; Value = $shVersion }
)) {
  if ($entry.Value -cne $version) { throw "$($entry.Label) version $($entry.Value) does not match Cargo.toml $version" }
}
if ($PSBoundParameters.ContainsKey('ReleaseTag') -and $ReleaseTag -cne "v$version") {
  throw "Release tag '$ReleaseTag' does not match package/bootstrap version v$version"
}
[pscustomobject]@{ Version = $version; ReleaseTag = $ReleaseTag; VerifiedFiles = 4 }
