# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Alexander Mohr
#
# Installs the Assimilate agent on this Windows host and enrolls it.
#
#   & ([scriptblock]::Create((irm https://SERVER/install.ps1))) `
#     -Server https://SERVER -Token enr_... -ServerKey KEY
#
# Run it from an administrator PowerShell. The enrollment token is single-use
# and short-lived; it is traded, over a channel only a server holding the key
# can complete, for the agent's own token.
param(
    [Parameter(Mandatory = $true)][string]$Server,
    [Parameter(Mandatory = $true)][string]$Token,
    [Parameter(Mandatory = $true)][string]$ServerKey,
    [switch]$SkipService,
    [switch]$Force
)

$ErrorActionPreference = 'Stop'
$Version = '@VERSION@'
$ReleasesUrl = if ($env:ASSIMILATE_RELEASES_URL) { $env:ASSIMILATE_RELEASES_URL } else { 'https://github.com/alexmohr/assimilate/releases/download' }
$InstallDir = Join-Path $env:ProgramFiles 'Assimilate'
$Server = $Server.TrimEnd('/')
if ($Server -like 'http://*') { Write-Warning "$Server is not HTTPS, so the downloads cannot be trusted" }

function Fail([string]$Message) {
    Write-Error "install.ps1: $Message"
    exit 1
}

$principal = [Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    Fail 'run this from an administrator PowerShell'
}
if ($Version -eq ('@' + 'VERSION@')) { Fail 'this script was not served by an Assimilate server' }
if ($env:PROCESSOR_ARCHITECTURE -ne 'AMD64') { Fail "unsupported architecture: $env:PROCESSOR_ARCHITECTURE" }
if ((Get-Service -Name 'assimilate-agent' -ErrorAction SilentlyContinue) -and -not $Force) {
    Fail 'this host already has the assimilate-agent service; use -Force to install again'
}

$name = "assimilate-agent-$Version-x86_64-pc-windows-msvc.zip"
$base = "$ReleasesUrl/v$Version"
$work = Join-Path ([IO.Path]::GetTempPath()) ([IO.Path]::GetRandomFileName())
New-Item -ItemType Directory -Path $work | Out-Null
try {
    Write-Host "==> Downloading $name"
    Invoke-WebRequest "$base/$name" -OutFile (Join-Path $work $name) -UseBasicParsing
    Invoke-WebRequest "$base/SHA256SUMS" -OutFile (Join-Path $work 'SHA256SUMS') -UseBasicParsing
    $line = Get-Content (Join-Path $work 'SHA256SUMS') | Where-Object { $_ -match " \*?$([regex]::Escape($name))$" } | Select-Object -First 1
    if (-not $line) { Fail "SHA256SUMS does not list $name" }
    $expected = ($line -split '\s+')[0]
    $actual = (Get-FileHash (Join-Path $work $name) -Algorithm SHA256).Hash
    if ($actual -ne $expected.ToUpperInvariant()) { Fail "the SHA-256 of $name is $actual, expected $expected" }
    Write-Host '==> SHA-256 verified'

    Expand-Archive (Join-Path $work $name) -DestinationPath $InstallDir -Force
} finally {
    Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
}

$agent = Join-Path $InstallDir 'assimilate-agent\assimilate-agent.exe'
Write-Host "==> Enrolling this host with $Server"
$env:BORG_ENROLLMENT_TOKEN = $Token
try {
    $agentToken = (& $agent --enroll --server-url $Server --server-key $ServerKey | Select-Object -First 1)
    if ($LASTEXITCODE -ne 0 -or -not $agentToken) { Fail 'enrollment failed; create a new token in the Add agent dialog and try again' }
} finally {
    Remove-Item Env:\BORG_ENROLLMENT_TOKEN -ErrorAction SilentlyContinue
}

if ($SkipService) {
    Write-Host '==> Skipping the service; the agent token was not stored'
    exit 0
}
$env:BORG_SERVER_URL = $Server
$env:BORG_SERVER_KEY = $ServerKey
$env:BORG_AGENT_TOKEN = $agentToken
try {
    & $agent service install
    if ($LASTEXITCODE -ne 0) { Fail 'installing the service failed' }
} finally {
    Remove-Item Env:\BORG_SERVER_URL, Env:\BORG_SERVER_KEY, Env:\BORG_AGENT_TOKEN -ErrorAction SilentlyContinue
}
Write-Host '==> The agent is running as a Windows service and starts at boot'
