# Dependency-free Windows PowerShell 5.1 regression tests using loopback HTTP.
$ErrorActionPreference = "Stop"
$selector = Join-Path $PSScriptRoot 'select-bootstrap-version.ps1'
$rules = [hashtable]::Synchronized(@{})
$requests = [Collections.ArrayList]::Synchronized([Collections.ArrayList]::new())
$state = [hashtable]::Synchronized(@{ Stop = $false; Failure = $null })
$listener = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, 0)
$listener.Start()
$baseUrl = "http://127.0.0.1:$($listener.LocalEndpoint.Port)"
$server = [PowerShell]::Create()
[void]$server.AddScript({
  param($listener, $rules, $requests, $state)
  try {
    while (-not $state.Stop) {
      if (-not $listener.Pending()) { [Threading.Thread]::Sleep(10); continue }
      $client = $listener.AcceptTcpClient()
      try {
        $stream = $client.GetStream()
        $stream.ReadTimeout = 5000
        $stream.WriteTimeout = 5000
        $reader = [IO.StreamReader]::new($stream)
        $request = $reader.ReadLine()
        if (-not $request) { continue }
        while ($reader.ReadLine()) { }
        $parts = $request.Split(' ')
        $key = "$($parts[0]) $($parts[1])"
        [void]$requests.Add($key)
        $spec = if ($rules.ContainsKey($key)) { $rules[$key] } else { @{ Status = 404; Body = 'missing' } }
        if ($spec.Disconnect) { continue }
        $body = [Text.Encoding]::UTF8.GetBytes([string]$spec.Body)
        $type = if ($spec.Type) { $spec.Type } else { 'text/plain; charset=utf-8' }
        $header = "HTTP/1.1 $($spec.Status) Fixture`r`nContent-Type: $type`r`nContent-Length: $($body.Length)`r`nConnection: close`r`n`r`n"
        $bytes = [Text.Encoding]::ASCII.GetBytes($header)
        $stream.Write($bytes, 0, $bytes.Length)
        if ($parts[0] -ne 'HEAD') { $stream.Write($body, 0, $body.Length) }
        $stream.Flush()
      } finally { $client.Close() }
    }
  } catch { if (-not $state.Stop) { $state.Failure = $_.Exception.Message } }
}).AddArgument($listener).AddArgument($rules).AddArgument($requests).AddArgument($state)
$handle = $server.BeginInvoke()
$passed = [Collections.Generic.List[string]]::new()
$fixtureParent = if ($env:RUNNER_TEMP) { $env:RUNNER_TEMP } else { [IO.Path]::GetTempPath() }
$fixtureParent = [IO.Path]::GetFullPath($fixtureParent)
$fixtureRoot = Join-Path $fixtureParent ("browser-cli-selector-" + [Guid]::NewGuid().ToString())
New-Item -ItemType Directory -Path $fixtureRoot | Out-Null

function Assert-Equal($Actual, $Expected) {
  if ($Actual -ne $Expected) { throw "Expected '$Expected', got '$Actual'" }
}
function Assert-Throws([scriptblock]$Body, [string]$Pattern) {
  try { & $Body | Out-Null } catch {
    if ($_.Exception.Message -match $Pattern) { return }
    throw
  }
  throw "Expected failure matching '$Pattern'"
}
function Set-Published([string]$Version) {
  $asset = "browser-cli-v$Version-x86_64-pc-windows-msvc.exe"
  $rules["GET /v$Version/SHA256SUMS"] = @{ Status = 200; Body = ('a' * 64) + "  $asset`n" }
  $rules["HEAD /v$Version/$asset"] = @{ Status = 200; Body = 'fixture' }
}
function Select-Version([string[]]$Tags) {
  & $selector -Tags $Tags -DownloadBaseUrl $baseUrl -WarningAction SilentlyContinue
}
function Test-Case([string]$Name, [scriptblock]$Body) {
  $rules.Clear()
  $requests.Clear()
  $tlsBefore = [Net.ServicePointManager]::SecurityProtocol
  & $Body
  Assert-Equal ([Net.ServicePointManager]::SecurityProtocol) $tlsBefore
  if ($state.Failure) { throw "Fixture server failed: $($state.Failure)" }
  $passed.Add($Name)
  Write-Host "PASS: $Name"
}

# Validate each exact deletion target, including the unchanged bootstrap's temp
# cleanup. This function exists only in this test process, not in the product.
function Remove-Item {
  [CmdletBinding()]
  param([Parameter(Position = 0)][string]$Path, [switch]$Recurse, [switch]$Force)
  $absolute = [IO.Path]::GetFullPath($Path)
  if (-not $absolute.StartsWith($fixtureRoot + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
    throw "Refusing cleanup outside the isolated test directory"
  }
  Microsoft.PowerShell.Management\Remove-Item -LiteralPath $absolute -Recurse:$Recurse -Force:$Force
}

try {
  Test-Case 'select newest available, not the second tag' {
    Set-Published '1.1.15'
    Set-Published '1.1.13'
    Assert-Equal (Select-Version @('v1.1.13', 'v1.1.14', 'v1.1.15')) '1.1.15'
    Assert-Equal $requests.Count 2
  }
  Test-Case 'tag before manifest publication falls back' {
    $rules['HEAD /v1.1.15/browser-cli-v1.1.15-x86_64-pc-windows-msvc.exe'] = @{ Status = 200 }
    Set-Published '1.1.14'
    Assert-Equal (Select-Version @('v1.1.15', 'v1.1.14')) '1.1.14'
    Assert-Equal $requests.Count 3
  }
  Test-Case 'manifest exists but binary 404 falls back past missing releases' {
    Set-Published '1.1.15'
    $rules.Remove('HEAD /v1.1.15/browser-cli-v1.1.15-x86_64-pc-windows-msvc.exe')
    Set-Published '1.1.13'
    Assert-Equal (Select-Version @('v1.1.15', 'v1.1.14', 'v1.1.13')) '1.1.13'
    Assert-Equal $requests.Count 5
  }
  Test-Case 'missing Windows checksum falls back' {
    $rules['GET /v1.1.15/SHA256SUMS'] = @{ Status = 200; Body = ('b' * 64) + '  linux-binary' }
    Set-Published '1.1.13'
    Assert-Equal (Select-Version @('v1.1.15', 'v1.1.13')) '1.1.13'
    Assert-Equal $requests.Count 3
  }
  Test-Case 'numeric ordering, duplicate tags and prerelease filtering' {
    Set-Published '1.10.0'
    Assert-Equal (Select-Version @('v1.9.0', 'v1.10.0', 'v1.10.0', 'v99.0.0-rc1', 'main', 'v1.2.3/evil')) '1.10.0'
    Assert-Equal $requests.Count 2
  }
  Test-Case 'GNU binary checksum marker and CRLF manifest' {
    Set-Published '1.1.15'
    $rules['GET /v1.1.15/SHA256SUMS'] = @{ Status = 200; Type = 'application/octet-stream'; Body =
      ('a' * 64) + " *browser-cli-v1.1.15-x86_64-pc-windows-msvc.exe`r`n" }
    Assert-Equal (Select-Version @('v1.1.15')) '1.1.15'
  }
  Test-Case 'empty tag list fails without HTTP requests' {
    Assert-Throws { Select-Version @() } 'No published Windows binary'
    Assert-Equal $requests.Count 0
  }
  Test-Case 'all candidates missing is a failure, not a skip' {
    Assert-Throws { Select-Version @('v1.1.15', 'v1.1.14') } 'No published Windows binary'
    Assert-Equal $requests.Count 2
  }
  Test-Case 'malformed checksum does not downgrade' {
    Set-Published '1.1.13'
    $rules['GET /v1.1.15/SHA256SUMS'] = @{ Status = 200; Body = 'bad-hash  browser-cli-v1.1.15-x86_64-pc-windows-msvc.exe' }
    Assert-Throws { Select-Version @('v1.1.15', 'v1.1.13') } 'Invalid checksum manifest'
    Assert-Equal $requests.Count 1
  }
  Test-Case 'duplicate checksum does not downgrade' {
    Set-Published '1.1.15'
    Set-Published '1.1.13'
    $rules['GET /v1.1.15/SHA256SUMS'].Body *= 2
    Assert-Throws { Select-Version @('v1.1.15', 'v1.1.13') } 'Invalid or duplicate checksum'
    Assert-Equal $requests.Count 1
  }
  foreach ($manifestBody in @('', '<html>upstream failure</html>')) {
    Test-Case "invalid manifest content [$manifestBody] does not downgrade" {
      Set-Published '1.1.13'
      $rules['GET /v1.1.15/SHA256SUMS'] = @{ Status = 200; Body = $manifestBody }
      Assert-Throws { Select-Version @('v1.1.15', 'v1.1.13') } 'Invalid checksum manifest'
      Assert-Equal $requests.Count 1
    }
  }
  Test-Case 'connection failure does not downgrade' {
    Set-Published '1.1.13'
    $rules['GET /v1.1.15/SHA256SUMS'] = @{ Disconnect = $true }
    Assert-Throws { Select-Version @('v1.1.15', 'v1.1.13') } '.'
    Assert-Equal @($requests | Where-Object { $_ -match '/v1.1.13/' }).Count 0
  }
  foreach ($status in @(403, 429, 500)) {
    Test-Case "manifest HTTP $status fails instead of downgrading" {
      Set-Published '1.1.13'
      $rules['GET /v1.1.15/SHA256SUMS'] = @{ Status = $status; Body = 'failure' }
      Assert-Throws { Select-Version @('v1.1.15', 'v1.1.13') } "$status"
      Assert-Equal $requests.Count 1
    }
  }
  foreach ($status in @(403, 500)) {
    Test-Case "binary HTTP $status fails instead of downgrading" {
      Set-Published '1.1.15'
      Set-Published '1.1.13'
      $rules['HEAD /v1.1.15/browser-cli-v1.1.15-x86_64-pc-windows-msvc.exe'] = @{ Status = $status; Body = 'failure' }
      Assert-Throws { Select-Version @('v1.1.15', 'v1.1.13') } "$status"
      Assert-Equal $requests.Count 2
    }
  }
  Test-Case 'real bootstrap rejects legacy release overrides without installing' {
    Set-Published '1.1.15'
    Set-Published '1.1.13'
    $rules['GET /v1.1.15/browser-cli-v1.1.15-x86_64-pc-windows-msvc.exe'] = @{ Status = 200; Body = 'corrupt binary' }
    $saved = @{}
    foreach ($name in @('LEXMOUNT_BROWSER_CLI_VERSION', 'LEXMOUNT_BROWSER_CLI_DOWNLOAD_BASE_URL', 'LEXMOUNT_BROWSER_CLI_INSTALL_DIR', 'TEMP', 'TMP')) {
      $saved[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
    }
    try {
      $env:LEXMOUNT_BROWSER_CLI_VERSION = Select-Version @('v1.1.15', 'v1.1.13')
      $env:LEXMOUNT_BROWSER_CLI_DOWNLOAD_BASE_URL = $baseUrl
      $env:LEXMOUNT_BROWSER_CLI_INSTALL_DIR = Join-Path $fixtureRoot 'install'
      $env:TEMP = $fixtureRoot
      $env:TMP = $fixtureRoot
      $repositoryRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
      Assert-Throws { & (Join-Path $repositoryRoot 'skills/lexmount-browser/scripts/bootstrap.ps1') } 'overrides are disabled'
      Assert-Equal (Test-Path -LiteralPath (Join-Path $env:LEXMOUNT_BROWSER_CLI_INSTALL_DIR 'browser-cli.exe')) $false
      Assert-Equal @($requests | Where-Object { $_ -match '^GET .*browser-cli-.*exe' }).Count 0
    } finally {
      foreach ($name in $saved.Keys) { [Environment]::SetEnvironmentVariable($name, $saved[$name], 'Process') }
    }
  }
  [pscustomobject]@{ passed = $passed.Count; powershell = $PSVersionTable.PSVersion.ToString(); cases = $passed } | ConvertTo-Json -Compress
} finally {
  $state.Stop = $true
  $listener.Stop()
  [void]$server.EndInvoke($handle)
  $server.Dispose()
  $absolute = [IO.Path]::GetFullPath($fixtureRoot)
  $parentPrefix = $fixtureParent.TrimEnd([IO.Path]::DirectorySeparatorChar) + [IO.Path]::DirectorySeparatorChar
  if (-not $absolute.StartsWith($parentPrefix, [StringComparison]::OrdinalIgnoreCase) -or
      (Split-Path -Leaf $absolute) -notmatch '^browser-cli-selector-[0-9a-f-]{36}$') {
    throw "Refusing cleanup of an unexpected fixture directory"
  }
  Microsoft.PowerShell.Management\Remove-Item -LiteralPath $absolute -Recurse -Force
}
