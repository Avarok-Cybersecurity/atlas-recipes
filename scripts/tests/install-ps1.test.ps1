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

    # The two installers must agree about a SUMS that names the same file twice.
    # They did not: sh printed every match and failed closed, ps1 kept the LAST
    # one and verified against it silently — so a release with a stale duplicate
    # would have been accepted on Windows and refused on unix.
    Set-Content -LiteralPath $sums -Value @(
        "$good  atlasctl-x86_64-pc-windows-msvc.zip"
        ("0" * 64 + "  atlasctl-x86_64-pc-windows-msvc.zip")
    )
    Should-Throw 'a conflicting duplicate entry is refused' 'more than once' {
        Assert-Checksum -Archive $archive -SumsFile $sums
    }

    # The same fact stated twice is still one fact.
    Set-Content -LiteralPath $sums -Value @(
        "$good  atlasctl-x86_64-pc-windows-msvc.zip"
        "$good  atlasctl-x86_64-pc-windows-msvc.zip"
    )
    try { Assert-Checksum -Archive $archive -SumsFile $sums; Ok 'an identical duplicate still verifies' }
    catch { Bad 'an identical duplicate still verifies' $_.Exception.Message }

    # `sha256sum -b` writes `*name`; sh accepted it and this did not, so a flag
    # added to the release pipeline would have killed every Windows install.
    Set-Content -LiteralPath $sums -Value "$good *atlasctl-x86_64-pc-windows-msvc.zip"
    try { Assert-Checksum -Archive $archive -SumsFile $sums; Ok 'a binary-mode entry verifies' }
    catch { Bad 'a binary-mode entry verifies' $_.Exception.Message }

    # A leading-whitespace line used to split into ['', 'sha  name'] and be
    # skipped, so an indented SUMS had no entry at all.
    Set-Content -LiteralPath $sums -Value "   $good  atlasctl-x86_64-pc-windows-msvc.zip"
    try { Assert-Checksum -Archive $archive -SumsFile $sums; Ok 'an indented entry still verifies' }
    catch { Bad 'an indented entry still verifies' $_.Exception.Message }

    # --- Die actually exits ---------------------------------------------------
    # This file REPLACES `Die` with a throw so a refusal is observable, which
    # puts production's exit path outside the tested surface by construction:
    # `Die` could stop exiting and nothing above would notice, while the
    # installer printed "Refusing to install" and then ran the binary. So run
    # the real one in a child pwsh and assert the status.
    $child = @"
`$ErrorActionPreference = 'Stop'
`$src = Get-Content -Raw -Encoding UTF8 -LiteralPath '$(Join-Path $root 'scripts/install.ps1')'
Invoke-Expression `$src.Substring(0, `$src.IndexOf('# --- main'))
Assert-Checksum -Archive '$archive' -SumsFile '$sums'
Write-Host 'REACHED-THE-LINE-AFTER'
"@
    Set-Content -LiteralPath $sums -Value ("0" * 64 + "  atlasctl-x86_64-pc-windows-msvc.zip")
    $childOut = & pwsh -NoProfile -NonInteractive -Command $child 2>&1 | Out-String
    if ($LASTEXITCODE -eq 1) { Ok 'a refusal exits non-zero' }
    else { Bad 'a refusal exits non-zero' "exit was $LASTEXITCODE" }
    if ($childOut -notmatch 'REACHED-THE-LINE-AFTER') { Ok 'a refusal stops the script' }
    else { Bad 'a refusal stops the script' 'execution continued past the refusal' }
    # And prove the child got as far as the CHECK. Exit 1 with no sentinel is
    # also what a harness that died on its own Get-Content looks like, so
    # without this the pair would pass while testing nothing.
    if ($childOut -match 'checksum mismatch') { Ok 'the child reached the checksum comparison' }
    else { Bad 'the child reached the checksum comparison' $childOut }

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
# Explicit BOTH ways. Falling off the end leaves the script's status as
# whatever the last command set — and one of the cases above deliberately runs
# a child pwsh that exits 1, so the suite reported "0 failed" and then failed
# the job with it.
if ($script:fail -gt 0) { exit 1 }
exit 0
