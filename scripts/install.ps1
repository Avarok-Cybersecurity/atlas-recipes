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
#
# To join a fleet at install time — the Windows counterpart of
# `curl … | sh -s -- --join <code>@<host>`:
#
#   & ([scriptblock]::Create((irm https://atlasinference.io/install.ps1))) -Join 12345678@10.0.0.1
#
# `irm | iex` cannot pass arguments; `[scriptblock]::Create` can, and is the
# idiom every Windows installer that takes options uses.

# `irm | iex` runs this in the CALLER'S SCOPE, so anything set here is still set
# in the operator's session afterwards. `$ErrorActionPreference` is saved and put
# back at the end for that reason: without it, a successful install silently left
# every later command in the window failing on the first non-terminating error.
$__atlasPrevEap = $ErrorActionPreference
$ErrorActionPreference = 'Stop'

# NOT `Set-StrictMode` here, for the same reason and one more: there is no way to
# read the caller's current strict mode, so it cannot be restored -- turning it
# off at the end would be a change in its own right for anyone who had it on. The
# test harness sets it before loading these functions, so the strictness this
# script is written under is still enforced where it can be observed, and not
# imposed on a session that did not ask for it.

# Deliberately NO `param()` block, and every option parsed by hand.
#
# `param([string]$Join, …)` makes `$Join` POSITIONAL, and PowerShell binds a
# `--anything` token to a positional parameter rather than treating it as a
# switch — so `--join CODE@HOST` bound `--join` itself to `$Join` and stranded
# the value, `--grant-control` alone BECAME the join target, and
# `--grant-control --join CODE@HOST` silently dropped the grant. The
# compatibility loop that was supposed to accept those spellings never saw
# them, because binding had already happened.
#
# With no param block, `$args` holds every token exactly as pasted and one
# parser handles both spellings. That matters because the shell installer, the
# docs and the site all say `--join`, and someone translating that line by hand
# will type it.
$Join = ''
$GrantControl = $false
$i = 0
while ($i -lt $args.Count) {
    $tok = [string]$args[$i]
    switch -Regex ($tok) {
        '^(--join|-Join)=(.+)$' { $Join = $Matches[2]; break }
        '^(--join|-Join)$' {
            if ($i + 1 -ge $args.Count) {
                # install.sh dies here rather than installing without the step
                # the operator came for. Silently skipping the join is how
                # someone walks away believing a machine joined a fleet.
                Write-Host "[atlas] error: --join needs a value, e.g. --join 12345678@10.10.10.1" -ForegroundColor Red
                exit 1
            }
            $i++; $Join = [string]$args[$i]; break
        }
        '^(--grant-control|-GrantControl)$' { $GrantControl = $true; break }
        default {
            Write-Host "[atlas] error: unrecognized option $tok" -ForegroundColor Red
            Write-Host "[atlas] usage: -Join <code>@<host> [-GrantControl]" -ForegroundColor Red
            exit 1
        }
    }
    $i++
}

$Repo    = 'Avarok-Cybersecurity/atlas-recipes'
$BinName = 'atlasctl'
$Port    = 34333

# Windows PowerShell 5.1 turns the FIRST stderr line of a native command into a
# TERMINATING error when $ErrorActionPreference is 'Stop'. Both probes below run
# commands that write to stderr in exactly the state they exist to DETECT --
# `docker info` when Docker Desktop is installed but its engine is stopped, and
# `agent status` when no agent is answering -- so under 5.1 the probe aborted the
# install instead of reporting, stranding the machine with a binary and no agent.
# pwsh 7 does not do this, which is why the test suite (which runs under pwsh)
# never saw it. Run the probe with the preference relaxed and judge it by its
# exit code, which is what the callers already wanted.
function Invoke-Probe {
    param([Parameter(Mandatory=$true)][scriptblock]$Probe)
    $prev = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try {
        & $Probe 2>&1 | Out-Null
        # A probe that never reached a native command leaves $LASTEXITCODE stale;
        # treating stale-as-zero would report a dead docker as healthy.
        if ($null -eq $LASTEXITCODE) { return 1 }
        return $LASTEXITCODE
    } catch {
        return 1
    } finally {
        $ErrorActionPreference = $prev
    }
}

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
    # -LiteralPath, like every other path this script touches: `Get-Content`
    # globs, and a directory named `[build]` is legal and would fail the
    # checksum lookup with "cannot find path" rather than a verdict.
    foreach ($line in Get-Content -LiteralPath $SumsFile) {
        # Trimmed first: a line with leading whitespace split into ['', 'sha  name']
        # and was skipped, so an indented SUMS silently had no entry at all.
        $parts = $line.Trim() -split '\s+', 2
        if ($parts.Count -ne 2) { continue }
        # `*name` is what `sha256sum -b` writes, and the shell installer accepts
        # it. Rejecting it here would kill every Windows install the day the
        # release pipeline adds a flag that unix does not notice.
        $entry = $parts[1].Trim()
        if ($entry -ne $name -and $entry -ne "*$name") { continue }
        $hash = $parts[0].Trim().ToLower()
        # A second, DIFFERENT hash for the same file is refused rather than
        # resolved. This loop used to keep overwriting, so a stale entry beside
        # a current one decided silently which bytes were acceptable — and the
        # two installers disagreed about which one won.
        if ($want -and $want -ne $hash) {
            Die "SHA256SUMS lists $name more than once, with different hashes. Refusing to install."
        }
        $want = $hash
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
    if (-not (Test-Path -LiteralPath $Exe)) { return '' }
    # A binary that cannot be run compares unequal, which lands on the upgrade
    # path -- the safe direction.
    try { return (& $Exe --version 2>$null | Select-Object -First 1) } catch { return '' }
}

# Should the downloaded binary replace the installed one? CONTENT, not version.
#
# `--version` reports the crate semver, which release tooling bumps only
# sometimes, so two builds separated by an entire wire-protocol revision can
# both report "atlasctl 0.1.7". A user hit exactly that: their agent spoke
# protocol 1, the published build spoke 4, both said 0.1.7, and the installer
# answered "already installed" every time while the control page kept sending
# them back to it. Bytes cannot lie about this.
function Test-BinaryDiffers {
    param($Installed, $Downloaded)
    if (-not (Test-Path -LiteralPath $Installed)) { return $true }
    try {
        return (Get-Sha256 $Installed) -ne (Get-Sha256 $Downloaded)
    } catch {
        # Unreadable installed binary: replacing it is the safe direction.
        return $true
    }
}

function Install-Agent {
    param($Exe, $SameVersion, $JoinTarget, [bool]$Grant)

    if ($env:ATLASCTL_NO_AGENT) {
        Write-Info "skipping the background agent (ATLASCTL_NO_AGENT is set)."
        Write-Info "start it yourself later with: $BinName agent install"
        return
    }

    # Re-running the installer on a machine that already has this exact version
    # is the commonest reason to run it at all: the binary is there, the task is
    # not running, and the website says "no agent". Nothing needs installing --
    # the machine needs starting.
    # A join is the step the operator came for, so it runs regardless of what
    # is already installed — the same rule the shell installer follows.
    if ($JoinTarget) {
        Write-Info "joining the fleet at $($JoinTarget -replace '^[^@]*@', '')"
        if ($Grant) { Write-Info "and letting that fleet run models on this machine" }
        $joinArgs = @('agent', 'install', '--join', $JoinTarget)
        if ($Grant) { $joinArgs += '--grant-control' }
        & $Exe @joinArgs
        if ($LASTEXITCODE -eq 0) { return }
        # One exit code covers two steps, so say plainly that either could have
        # failed and give the check that tells them apart — rather than
        # asserting the install worked and only the join did not, which sends
        # operators to mint a fresh code for a task that was never created.
        Write-Warn "the agent install, the fleet join, or both did not succeed."
        Write-Warn "Check which with:  $BinName agent status"
        Write-Warn "A join code is single-use and expires. To retry just the join:"
        Write-Warn "    $BinName agent install --join <code>@<host>"
        return
    }

    if ($SameVersion) {
        $answering = (Invoke-Probe { & $Exe agent status }) -eq 0
        # BOTH, not just the port. An operator who ran `atlasctl agent run` by
        # hand has an agent answering and NO task, and skipping on the port
        # alone left it that way: close the window, reboot, and "the website
        # says no agent" is back -- the symptom this script exists to fix,
        # reached through another door.
        $supervised = $null -ne (Get-ScheduledTask -TaskName 'atlasctl-agent' -ErrorAction SilentlyContinue)
        if ($answering -and $supervised) {
            Write-Info "the agent is already running as a task - this machine is ready."
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
    if ((Invoke-Probe { docker info }) -ne 0) {
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
    if (-not (Test-Path -LiteralPath $staged)) { Die "the archive did not contain $BinName.exe" }

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
    # Versions are for the OPERATOR to read; the decision is content -- see
    # Test-BinaryDiffers.
    $differs = Test-BinaryDiffers -Installed $exe -Downloaded $staged
    $sameVersion = -not $differs

    if ($sameVersion) {
        Write-Info "$newVersion is already installed here - keeping it."
    } else {
        if ($oldVersion -and $oldVersion -eq $newVersion) {
            # Same version string, different build: say so rather than printing
            # "upgrading 0.1.7 -> 0.1.7", which reads like a bug.
            Write-Info "$oldVersion is installed, but the published build differs - replacing it."
        } elseif ($oldVersion) {
            Write-Info "upgrading $oldVersion -> $newVersion"
        }
        # Replacing a RUNNING exe fails with "file in use", and the agent this
        # installer is upgrading is exactly such a process. Move it aside first:
        # Windows permits renaming a running image, and the stale copy is
        # cleaned up on the next install.
        $old = "$exe.old"
        $moved = $false
        if (Test-Path -LiteralPath $exe) {
            Remove-Item -Force $old -ErrorAction SilentlyContinue
            try { Move-Item -Force $exe $old; $moved = $true } catch { }
        }
        try {
            Copy-Item -Force $staged $exe
        } catch {
            # Without this the machine is left with NO binary at all: the working
            # exe is stranded at .old and the scheduled task points at a path that
            # no longer exists. An upgrade that fails must leave the old agent
            # running, not take the fleet's node offline.
            if ($moved) {
                Move-Item -Force $old $exe
                Die "could not install the new binary ($($_.Exception.Message)); kept $oldVersion"
            }
            throw
        }
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
    Install-Agent -Exe $exe -SameVersion $sameVersion -JoinTarget $Join -Grant $GrantControl

    Write-Info "done. Try:"
    Write-Info "    $BinName list"
    Write-Info "    $BinName run qwen3.6-35b-a3b-fp8-mtp"
} finally {
    Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
    # Hand the session back as it was found. This runs on the success path and on
    # a thrown failure; it does NOT run when `Die` calls `exit`, which is the
    # right trade -- the operator is looking at an error message there, and
    # restructuring the whole script to catch that case would put `exit`
    # semantics inside a script block, where a failed checksum might stop halting
    # the install. That is a far worse bug than a preference left set.
    $ErrorActionPreference = $__atlasPrevEap
}
