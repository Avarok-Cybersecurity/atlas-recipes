# SPDX-License-Identifier: AGPL-3.0-only
#
# Tests for scripts/install.ps1 — the `irm … | iex` one-liner.
#
# The counterpart of install-sh.test.sh, over the same three decisions: which
# target, whether a downloaded binary may be executed, and what to do about an
# agent that may or may not already be there. Written without Pester so it runs
# on a bare windows-latest with nothing installed.
#
# The script's functions are loaded by taking everything ABOVE its `# --- main`
# marker and evaluating that, rather than by adding a "do not run when dot
# sourced" guard to install.ps1. A hook that exists only for tests is
# test-specific code in a production path; this file is where that cost belongs.

$ErrorActionPreference = 'Stop'

$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$src = Get-Content -Raw -Encoding UTF8 -LiteralPath (Join-Path $root 'scripts/install.ps1')
$cut = $src.IndexOf('# --- main')
if ($cut -lt 0) { throw 'install.ps1 no longer has its `# --- main` marker; this loader needs updating' }
Invoke-Expression $src.Substring(0, $cut)

# `Die` calls `exit`, which would end the test host rather than the case under
# test. Redefined to throw so a refusal is observable instead of fatal.
function Die { param($m) throw $m }

$script:pass = 0
$script:fail = 0
function Ok   { param($n) $script:pass++; Write-Host "  ok   $n" }
function Bad  { param($n, $d) $script:fail++; Write-Host "  FAIL $n`n     $d" }
function Should-Contain { param($n, $hay, $needle)
    if ("$hay" -like "*$needle*") { Ok $n } else { Bad $n "[$hay] does not contain [$needle]" }
}
function Should-Throw { param($n, $needle, [scriptblock]$body)
    try { & $body; Bad $n "expected a refusal mentioning [$needle], got none" }
    catch { Should-Contain $n $_.Exception.Message $needle }
}

# --- Assert-Checksum ----------------------------------------------------------
# The decision that stands between `irm | iex` and executing arbitrary bytes.
$work = Join-Path ([System.IO.Path]::GetTempPath()) ("atlasctl-ps1-test-" + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force -Path $work | Out-Null
try {
    $archive = Join-Path $work 'atlasctl-x86_64-pc-windows-msvc.zip'
    Set-Content -LiteralPath $archive -Value 'payload' -NoNewline
    $good = (Get-FileHash -Algorithm SHA256 -LiteralPath $archive).Hash.ToLower()

    $sums = Join-Path $work 'SHA256SUMS'
    Set-Content -LiteralPath $sums -Value "$good  atlasctl-x86_64-pc-windows-msvc.zip"
    try { Assert-Checksum -Archive $archive -SumsFile $sums; Ok 'a matching checksum is accepted' }
    catch { Bad 'a matching checksum is accepted' $_.Exception.Message }

    Set-Content -LiteralPath $sums -Value ("0" * 64 + "  atlasctl-x86_64-pc-windows-msvc.zip")
    Should-Throw 'a mismatched checksum refuses to install' 'checksum mismatch' {
        Assert-Checksum -Archive $archive -SumsFile $sums
    }

    Set-Content -LiteralPath $sums -Value "$good  some-other-file.zip"
    Should-Throw 'an archive with no entry refuses to install' 'no entry' {
        Assert-Checksum -Archive $archive -SumsFile $sums
    }

    # The shell script matched the filename as a regex, so a line naming a
    # DIFFERENT file satisfied the lookup. This one splits on whitespace and
    # compares exactly; the case is pinned here so the two stay agreed.
    Set-Content -LiteralPath $sums -Value "$good  atlasctl-x86_64-pc-windows-msvcXzip"
    Should-Throw 'a name that only matches loosely is not accepted' 'no entry' {
        Assert-Checksum -Archive $archive -SumsFile $sums
    }

    # --- Get-Version ----------------------------------------------------------
    # An unreadable or absent binary must compare UNEQUAL, which lands on the
    # upgrade path — the safe direction. Reporting a version for a binary that
    # cannot run would skip the install entirely.
    if ((Get-Version (Join-Path $work 'does-not-exist.exe')) -eq '') {
        Ok 'a missing binary reports no version'
    } else {
        Bad 'a missing binary reports no version' 'got something'
    }
} finally {
    Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
}

# --- Get-Target ---------------------------------------------------------------
# Reads OSArchitecture, so it cannot be stubbed without rewriting the function.
# What IS worth pinning is that this machine resolves to something buildable,
# and that the ARM refusal names itself rather than falling through to an
# x86_64 download that would run under emulation against an untested Docker.
$t = Get-Target
Should-Contain 'this runner resolves to a real target' $t 'pc-windows-msvc'

Write-Host ""
Write-Host "  $script:pass passed, $script:fail failed"
if ($script:fail -gt 0) { exit 1 }
