# SPDX-License-Identifier: AGPL-3.0-only
#
# atlasctl installer for Windows.
#
#   irm https://atlasinference.io/install.ps1 | iex
#
# The counterpart of scripts/install.sh, and deliberately the same SHAPE: same
# resolution of "latest", same refusal to install a binary whose checksum is
# not in SHA256SUMS, and the same three outcomes — fresh install, upgrade, or
# "already at this version, so start what is already here". Anyone comparing
# the two should find the arguments identical and only the syntax different.
#
# This script deliberately embeds no version. It always resolves what the
# release marked latest, so a stale copy of the script cannot pin an old
# release.

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$Repo    = 'Avarok-Cybersecurity/atlas-recipes'
$BinName = 'atlasctl'
$Port    = 34333

function Write-Info { param($m) Write-Host "[atlas] $m" -ForegroundColor Cyan }
function Write-Warn { param($m) Write-Host "[atlas] $m" -ForegroundColor Yellow }
function Die { param($m) Write-Host "[atlas] error: $m" -ForegroundColor Red; exit 1 }

function Get-Target {
    # PROCESSOR_ARCHITECTURE is the *process* architecture, which reads AMD64
    # for a 64-bit PowerShell on an ARM machine running emulation. The OS
    # architecture is the one that decides which binary can run natively.
    $arch = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture
    switch ($arch) {
        'X64'   { return 'x86_64-pc-windows-msvc' }
        'Arm64' {
            Die ("Windows on ARM is not supported yet: no atlasctl binary is " +
                 "built for it. x86_64 emulation would run, but Docker Desktop " +
                 "is what atlasctl drives and that combination is untested, so " +
                 "shipping it would be shipping a promise.")
        }
        default { Die "unsupported processor architecture: $arch" }
    }
}

function Get-Sha256 { param($Path) (Get-FileHash -Algorithm SHA256 -Path $Path).Hash.ToLower() }

# The one check that has to hold before anything is executed: the bytes match
# what the release says they are.
function Assert-Checksum {
    param($Archive, $SumsFile)
    $name = Split-Path -Leaf $Archive
    $want = $null
    foreach ($line in Get-Content $SumsFile) {
        $parts = $line -split '\s+', 2
        if ($parts.Count -eq 2 -and $parts[1].Trim() -eq $name) { $want = $parts[0].Trim().ToLower() }
    }
    if (-not $want) { Die "SHA256SUMS has no entry for $name; refusing to install unverified binaries." }
    $have = Get-Sha256 $Archive
    if ($have -ne $want) {
        Die "checksum mismatch for ${name}: expected $want, got $have. Refusing to install."
    }
    Write-Info "checksum verified"
}

function Get-Version {
    param($Exe)
    if (-not (Test-Path $Exe)) { return '' }
    # A binary that cannot be run compares unequal, which lands on the upgrade
    # path -- the safe direction.
    try { return (& $Exe --version 2>$null | Select-Object -First 1) } catch { return '' }
}

function Install-Agent {
    param($Exe, $SameVersion)

    if ($env:ATLASCTL_NO_AGENT) {
        Write-Info "skipping the background agent (ATLASCTL_NO_AGENT is set)."
        Write-Info "start it yourself later with: $BinName agent install"
        return
    }

    # Re-running the installer on a machine that already has this exact version
    # is the commonest reason to run it at all: the binary is there, the task is
    # not running, and the website says "no agent". Nothing needs installing --
    # the machine needs starting.
    if ($SameVersion) {
        & $Exe agent status *> $null
        if ($LASTEXITCODE -eq 0) {
            Write-Info "the agent is already running - this machine is ready."
            Write-Info "if a browser does not see it, pair it: $BinName agent token"
            return
        }
        Write-Info "starting the agent that is already installed here"
    } else {
        Write-Info "installing the background agent"
    }

    & $Exe agent install
    if ($LASTEXITCODE -eq 0) {
        Write-Info "pair your browser with: $BinName agent token"
        return
    }

    Write-Warn "could not install the agent as a scheduled task on this machine."
    Write-Warn "atlasctl itself is installed and works."
    # Naming the likeliest cause rather than telling the operator to re-run the
    # command that just failed: an agent already holding the port is what makes
    # a re-install look like a broken install.
    $held = Get-NetTCPConnection -LocalPort $Port -State Listen -ErrorAction SilentlyContinue
    if ($held) {
        Write-Warn "an agent is ALREADY listening on 127.0.0.1:$Port, so this machine"
        Write-Warn "is usable right now - the website will find it. Nothing else to do."
        return
    }
    Write-Warn "To run the agent by hand:"
    Write-Warn "    $BinName agent run"
    Write-Warn "Its startup log, if the task did start and then exit:"
    Write-Warn "    Get-Content -Tail 50 $env:LOCALAPPDATA\atlasctl\atlasctl-agent.log"
}

function Test-Docker {
    if (-not (Get-Command docker -ErrorAction SilentlyContinue)) {
        Write-Warn "docker was not found. ``atlasctl run`` needs it; ``atlasctl list`` and"
        Write-Warn "the control page work without it. Install Docker Desktop from"
        Write-Warn "https://docs.docker.com/desktop/install/windows-install/"
        return
    }
    docker info *> $null
    if ($LASTEXITCODE -ne 0) {
        # Installed but not running is its own state, and its own fix. Telling
        # someone to install what they already have is how a working machine
        # gets called broken.
        Write-Warn "docker is installed but not responding. Start Docker Desktop and"
        Write-Warn "wait for it to say 'Engine running', then try ``atlasctl run`` again."
    }
}

# --- main ---------------------------------------------------------------------

$target = Get-Target

$version = if ($env:ATLASCTL_VERSION) { $env:ATLASCTL_VERSION } else { 'latest' }
$base = if ($version -eq 'latest') {
    "https://github.com/$Repo/releases/latest/download"
} else {
    "https://github.com/$Repo/releases/download/$version"
}

$tmp = Join-Path ([System.IO.Path]::GetTempPath()) ("atlasctl-" + [System.Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force -Path $tmp | Out-Null
try {
    $archiveName = "$BinName-$target.zip"
    Write-Info "downloading $archiveName"
    $archive = Join-Path $tmp $archiveName
    try {
        Invoke-WebRequest -UseBasicParsing -Uri "$base/$archiveName" -OutFile $archive
    } catch {
        Die "could not download $base/$archiveName - is there a release for $target yet?"
    }
    $sums = Join-Path $tmp 'SHA256SUMS'
    try {
        Invoke-WebRequest -UseBasicParsing -Uri "$base/SHA256SUMS" -OutFile $sums
    } catch {
        Die "could not download SHA256SUMS; refusing to install unverified binaries."
    }
    Assert-Checksum -Archive $archive -SumsFile $sums

    Expand-Archive -Path $archive -DestinationPath $tmp -Force
    $staged = Join-Path $tmp "$BinName.exe"
    if (-not (Test-Path $staged)) { Die "the archive did not contain $BinName.exe" }

    $dir = if ($env:ATLASCTL_INSTALL_DIR) {
        $env:ATLASCTL_INSTALL_DIR
    } else {
        Join-Path $env:LOCALAPPDATA 'Programs\atlasctl'
    }
    New-Item -ItemType Directory -Force -Path $dir | Out-Null
    $exe = Join-Path $dir "$BinName.exe"

    # Ask the two binaries their versions rather than parsing a release tag: the
    # downloaded one is already here and already checksum-verified, so this
    # needs no second endpoint and cannot be fooled by a tag naming scheme that
    # changes.
    $newVersion = Get-Version $staged
    $oldVersion = Get-Version $exe
    $sameVersion = $newVersion -and ($oldVersion -eq $newVersion)

    if ($sameVersion) {
        Write-Info "$newVersion is already installed here - keeping it."
    } else {
        if ($oldVersion) { Write-Info "upgrading $oldVersion -> $newVersion" }
        # Replacing a RUNNING exe fails with "file in use", and the agent this
        # installer is upgrading is exactly such a process. Move it aside first:
        # Windows permits renaming a running image, and the stale copy is
        # cleaned up on the next install.
        if (Test-Path $exe) {
            $old = "$exe.old"
            Remove-Item -Force $old -ErrorAction SilentlyContinue
            try { Move-Item -Force $exe $old } catch { }
        }
        Copy-Item -Force $staged $exe
        Write-Info "installed $exe"
    }

    # User PATH, not machine PATH: this install needs no administrator, and
    # writing the machine PATH from a non-elevated shell fails anyway.
    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    if (-not $userPath) { $userPath = '' }
    if (($userPath -split ';') -notcontains $dir) {
        [Environment]::SetEnvironmentVariable('Path', "$userPath;$dir".Trim(';'), 'User')
        # The variable above only reaches NEW processes, so say so rather than
        # letting the next command in this very window fail as "not recognized".
        Write-Info "added $dir to your PATH - open a new terminal for it to take effect."
        $env:Path = "$env:Path;$dir"
    }

    Test-Docker
    Install-Agent -Exe $exe -SameVersion $sameVersion

    Write-Info "done. Try:"
    Write-Info "    $BinName list"
    Write-Info "    $BinName run qwen3.6-35b-a3b-fp8-mtp"
} finally {
    Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
}
