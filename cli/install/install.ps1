# Install iohr, the InOrbit command line, for this user (platform RFC 0021, ADR 0010).
#
#   powershell -c "irm https://packages.inorbit.hr/install.ps1 | iex"
#
# What it does, and nothing else:
#   1. picks the archive for this machine (Windows x86-64 or arm64);
#   2. downloads it and the release's SHA256SUMS from the GitHub release over HTTPS;
#   3. refuses unless the archive's SHA-256 matches SHA256SUMS, and, when the GitHub CLI
#      is installed and signed in, unless `gh attestation verify` confirms this
#      repository's release workflow built it;
#   4. copies iohr.exe into %LOCALAPPDATA%\Programs\iohr (no administrator rights) and adds
#      that directory to your user PATH, saying so.
#
# Settings, all optional (environment variables, since `irm | iex` takes no arguments):
#   IOHR_VERSION=0.1.0-alpha.1   a version (default: the newest iohr release)
#   IOHR_INSTALL_DIR=C:\some\dir where to put iohr.exe

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

$repo = 'inorbithr/sdk'
$dir = if ($env:IOHR_INSTALL_DIR) { $env:IOHR_INSTALL_DIR } else { Join-Path $env:LOCALAPPDATA 'Programs\iohr' }
$version = $env:IOHR_VERSION

function Say([string]$message) { Write-Host "iohr install: $message" }
function Fail([string]$message) { throw "iohr install: error: $message" }

# The machine's architecture, also from a 32-bit or emulated PowerShell.
$machine = if ($env:PROCESSOR_ARCHITEW6432) { $env:PROCESSOR_ARCHITEW6432 } else { $env:PROCESSOR_ARCHITECTURE }
switch ($machine) {
  'AMD64' { $arch = 'x86_64' }
  'ARM64' { $arch = 'aarch64' }
  default { Fail "no build for $machine" }
}
$target = "$arch-pc-windows-msvc"

$tmp = Join-Path ([IO.Path]::GetTempPath()) ("iohr-install-" + [Guid]::NewGuid())
New-Item -ItemType Directory -Path $tmp | Out-Null
try {
  if (-not $version) {
    Say 'finding the newest iohr release'
    $releases = Invoke-RestMethod -UseBasicParsing "https://api.github.com/repos/$repo/releases?per_page=100"
    $tag = $releases | Where-Object { $_.tag_name -like 'iohr/v*' } | Select-Object -First 1
    if (-not $tag) { Fail 'no iohr release found' }
    $version = $tag.tag_name.Substring('iohr/v'.Length)
  }
  if ($version -notmatch '^[0-9A-Za-z.-]+$') { Fail "not a version: $version" }

  $file = "iohr-$version-$target.zip"
  $base = "https://github.com/$repo/releases/download/iohr/v$version"
  Say "installing iohr $version for $target into $dir"
  Say "downloading $base/$file"
  Invoke-WebRequest -UseBasicParsing "$base/$file" -OutFile (Join-Path $tmp $file)
  Invoke-WebRequest -UseBasicParsing "$base/SHA256SUMS" -OutFile (Join-Path $tmp 'SHA256SUMS')

  $want = Get-Content (Join-Path $tmp 'SHA256SUMS') |
    Where-Object { ($_ -split '\s+')[1] -in @($file, "*$file") } |
    ForEach-Object { ($_ -split '\s+')[0] } | Select-Object -First 1
  if (-not $want) { Fail "$file is not in SHA256SUMS" }
  $got = (Get-FileHash -Algorithm SHA256 (Join-Path $tmp $file)).Hash.ToLowerInvariant()
  if ($got -ne $want.ToLowerInvariant()) { Fail "checksum mismatch for $file (expected $want, got $got); nothing was installed" }
  Say "checksum ok ($got)"

  # Windows PowerShell 5.1 turns a native command's stderr into an error under 'Stop'.
  $ghReady = $false
  if (Get-Command gh -ErrorAction SilentlyContinue) {
    try { & gh auth status *> $null; $ghReady = ($LASTEXITCODE -eq 0) } catch { $ghReady = $false }
  }
  if ($ghReady) {
    & gh attestation verify (Join-Path $tmp $file) --repo $repo | Out-Null
    if ($LASTEXITCODE -ne 0) { Fail "gh attestation verify failed for $file; nothing was installed" }
    Say "attestation ok (built by $repo's release workflow)"
  } else {
    Say 'attestation not checked (needs the GitHub CLI, signed in); the checksum was'
  }

  Expand-Archive -Path (Join-Path $tmp $file) -DestinationPath $tmp
  New-Item -ItemType Directory -Force -Path $dir | Out-Null
  Copy-Item -Force (Join-Path $tmp "iohr-$version-$target\iohr.exe") (Join-Path $dir 'iohr.exe')
  Say "installed $(Join-Path $dir 'iohr.exe')"
  & (Join-Path $dir 'iohr.exe') --version

  $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
  if (-not (($userPath -split ';') -contains $dir)) {
    $newPath = if ($userPath) { "$userPath;$dir" } else { $dir }
    [Environment]::SetEnvironmentVariable('Path', $newPath, 'User')
    Say "added $dir to your user PATH; open a new terminal to use it"
  }
  Say 'next: iohr login'
} finally {
  Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
}
