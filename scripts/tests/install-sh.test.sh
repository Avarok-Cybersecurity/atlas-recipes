#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-only
# Every `.` in this file sources $WORK/lib.sh, generated at run time from
# install.sh below — there is no path shellcheck could follow. A file-level
# directive has to precede all code, which is why it sits up here rather than
# beside the sources it is about.
# shellcheck disable=SC1091
#
# Tests for scripts/install.sh — the `curl … | sh` one-liner.
#
# It is the most-executed artifact this project ships and the only one that
# runs on a machine before any of our code is trusted, and it had no coverage
# at all beyond shellcheck. The two things worth pinning are the decision to
# EXECUTE a downloaded binary (verify_checksum) and the decisions an operator
# actually hits (which target, and what `install_agent` does about an agent
# that may or may not already be there).
#
# install.sh is loaded by stripping its final `main "$@"` rather than by adding
# a "don't run when sourced" guard to it: a hook that exists only for tests is
# test-specific code in a production path, and this file is the right place to
# pay that cost instead.

set -u


ROOT=$(cd "$(dirname "$0")/../.." && pwd)
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT INT TERM

# Refuse rather than proceed if that line is not exactly what we expect. `sed`
# silently strips nothing when the pattern misses — say the line gains a
# trailing comment — and the first `. "$WORK/lib.sh"` below would then run the
# REAL installer: a download from GitHub and an install into ~/.local/bin, on
# whatever machine happens to be running the tests.
grep -qxF 'main "$@"' "$ROOT/scripts/install.sh" || {
    echo "install.sh no longer ends with a bare \`main \"\$@\"\`; this loader would"
    echo "source it and RUN the installer. Update the loader before the tests."
    exit 1
}
sed 's/^main "\$@"$//' "$ROOT/scripts/install.sh" > "$WORK/lib.sh"
# Belt and braces: prove the line is gone from what we are about to source.
grep -qxF 'main "$@"' "$WORK/lib.sh" && { echo "the entrypoint survived the strip"; exit 1; }

pass=0; fail=0
ok()   { pass=$((pass+1)); printf '  ok   %s\n' "$1"; }
bad()  { fail=$((fail+1)); printf '  FAIL %s\n     %s\n' "$1" "$2"; }
check() { # name expected actual
    if [ "$2" = "$3" ]; then ok "$1"; else bad "$1" "expected [$2], got [$3]"; fi
}
contains() { # name haystack needle
    case "$2" in *"$3"*) ok "$1" ;; *) bad "$1" "[$2] does not contain [$3]" ;; esac
}

# --- detect_target ------------------------------------------------------------
# Runs in a subshell per case so the stubbed `uname` cannot leak.
target_for() { # os arch
    ( . "$WORK/lib.sh"
      # shellcheck disable=SC2317  # called indirectly, by detect_target
      uname() { if [ "$1" = "-s" ]; then echo "$OS"; else echo "$ARCH"; fi; }
      OS="$1" ARCH="$2" detect_target ) 2>&1
}

check "linux x86_64"       "x86_64-unknown-linux-musl"  "$(OS=Linux ARCH=x86_64 target_for Linux x86_64)"
check "macos arm"          "aarch64-apple-darwin"       "$(target_for Darwin arm64)"
check "linux aarch64"      "aarch64-unknown-linux-musl" "$(target_for Linux aarch64)"
check "linux amd64 alias"  "x86_64-unknown-linux-musl"  "$(target_for Linux amd64)"

# Git Bash reports MINGW64_NT-…: the operator is on Windows and there IS an
# installer for them, so pointing at it beats "not supported".
contains "git bash names the powershell one-liner" "$(target_for MINGW64_NT-10.0-22631 x86_64)" "install.ps1"
contains "an unknown arch is refused by name"      "$(target_for Linux mips64)"                 "mips64"

# --- verify_checksum ----------------------------------------------------------
printf 'payload\n' > "$WORK/atlasctl-x86_64-unknown-linux-musl.tar.xz"
good=$(sha256sum "$WORK/atlasctl-x86_64-unknown-linux-musl.tar.xz" | awk '{print $1}')

# Captures the STATUS as well as the output. Asserting only on the message
# means `die` could lose its `exit 1` and every case here would still pass —
# while production printed "Refusing to install" and then installed.
verify() { ( . "$WORK/lib.sh"; verify_checksum "$1" "$2" ) 2>&1; }
verify_rc() { ( . "$WORK/lib.sh"; verify_checksum "$1" "$2" ) >/dev/null 2>&1; echo $?; }

printf '%s  atlasctl-x86_64-unknown-linux-musl.tar.xz\n' "$good" > "$WORK/SUMS.good"
contains "a matching checksum is accepted" \
    "$(verify "$WORK/atlasctl-x86_64-unknown-linux-musl.tar.xz" "$WORK/SUMS.good")" "checksum verified"

printf '%s  atlasctl-x86_64-unknown-linux-musl.tar.xz\n' "${good%??}00" > "$WORK/SUMS.bad"
contains "a mismatched checksum refuses to install" \
    "$(verify "$WORK/atlasctl-x86_64-unknown-linux-musl.tar.xz" "$WORK/SUMS.bad")" "Refusing to install"

printf '%s  some-other-file.tar.xz\n' "$good" > "$WORK/SUMS.absent"
contains "an archive with no entry refuses to install" \
    "$(verify "$WORK/atlasctl-x86_64-unknown-linux-musl.tar.xz" "$WORK/SUMS.absent")" "no checksum for"

# A SUMS naming the same file twice with different hashes must be refused, not
# resolved. Taking either end silently lets a stale entry beside a current one
# decide which bytes are acceptable — and the two installers took DIFFERENT
# ends, so a release with a duplicate would have verified differently per OS.
printf '%s  atlasctl-x86_64-unknown-linux-musl.tar.xz\n%s  atlasctl-x86_64-unknown-linux-musl.tar.xz\n' \
    "$good" "${good%??}00" > "$WORK/SUMS.dup"
contains "a conflicting duplicate entry is refused" \
    "$(verify "$WORK/atlasctl-x86_64-unknown-linux-musl.tar.xz" "$WORK/SUMS.dup")" "more than once"

# An identical duplicate is harmless and must NOT be refused: the same fact
# stated twice is still one fact, and failing there would break a release for a
# cosmetic flaw in its sums file.
printf '%s  atlasctl-x86_64-unknown-linux-musl.tar.xz\n%s  atlasctl-x86_64-unknown-linux-musl.tar.xz\n' \
    "$good" "$good" > "$WORK/SUMS.dupsame"
contains "an identical duplicate still verifies" \
    "$(verify "$WORK/atlasctl-x86_64-unknown-linux-musl.tar.xz" "$WORK/SUMS.dupsame")" "checksum verified"

# `sha256sum -b` writes `*name`. The shell installer accepted it and the
# PowerShell one did not, so a flag added to the release pipeline would have
# killed every Windows install while unix carried on.
printf '%s *atlasctl-x86_64-unknown-linux-musl.tar.xz\n' "$good" > "$WORK/SUMS.binmode"
contains "a binary-mode entry verifies" \
    "$(verify "$WORK/atlasctl-x86_64-unknown-linux-musl.tar.xz" "$WORK/SUMS.binmode")" "checksum verified"

# A refusal has to STOP the script, not merely say so. Every assertion above is
# about output; if `die` lost its `exit 1` they would all still pass while the
# installer went on to run an unverified binary.
check "a mismatched checksum exits non-zero" "1" \
    "$(verify_rc "$WORK/atlasctl-x86_64-unknown-linux-musl.tar.xz" "$WORK/SUMS.bad")"
check "a missing entry exits non-zero" "1" \
    "$(verify_rc "$WORK/atlasctl-x86_64-unknown-linux-musl.tar.xz" "$WORK/SUMS.absent")"
check "a good checksum exits zero" "0" \
    "$(verify_rc "$WORK/atlasctl-x86_64-unknown-linux-musl.tar.xz" "$WORK/SUMS.good")"
# And the refusal must not be followed by the success line, which is what a
# `die` that printed and returned would look like.
case "$(verify "$WORK/atlasctl-x86_64-unknown-linux-musl.tar.xz" "$WORK/SUMS.bad")" in
    *"checksum verified"*) bad "a refusal must not also report success" "it did" ;;
    *) ok "a refusal must not also report success" ;;
esac

# The regression: the name was interpolated into a REGEX, so `.` matched any
# character and a line naming a DIFFERENT file satisfied the lookup — handing
# back a hash for bytes nobody checked.
printf '%s  atlasctl-x86_64-unknown-linux-muslXtarXxz\n' "$good" > "$WORK/SUMS.regex"
contains "a name that only matches as a regex is not accepted" \
    "$(verify "$WORK/atlasctl-x86_64-unknown-linux-musl.tar.xz" "$WORK/SUMS.regex")" "no checksum for"

# --- verify_attestation -------------------------------------------------------
# "Cannot check" and "checked, and it does not verify" are different facts, and
# only the second is alarming. Reporting them identically meant every machine
# with a gh older than 2.49 saw a security warning on a healthy install — which
# is how an operator learns to scroll past the real one.
attest() { # gh_state
    ( . "$WORK/lib.sh"
      # shellcheck disable=SC2317  # called indirectly, by verify_attestation
      case "$1" in
        absent)   command() { if [ "$2" = gh ]; then return 1; fi; /usr/bin/env command "$@"; } ;;
        old)      gh() { case "$1" in attestation) return 1 ;; *) return 0 ;; esac; } ;;
        # capable, but never `gh auth login` — the state a machine is in right
        # after `apt install gh`, and the one a pre-check silently skipped.
        loggedout) gh() { case "$1$2" in attestation--help) return 0 ;; \
                     attestationverify) echo "gh: To get started with GitHub CLI, please run: gh auth login"; return 1 ;; \
                     *) return 1 ;; esac; } ;;
        # capable and signed in, but GitHub is unreachable. A pre-check cannot
        # see this case at all, and it must not alarm anyone.
        offline)  gh() { case "$1$2" in attestation--help) return 0 ;; \
                     attestationverify) echo "dial tcp: lookup api.github.com: no such host"; return 1 ;; \
                     *) return 0 ;; esac; } ;;
        # capable, signed in, reached GitHub, and the answer was no.
        broken)   gh() { case "$1$2" in attestation--help) return 0 ;; \
                     attestationverify) echo "verification failed: no matching attestation found"; return 1 ;; \
                     *) return 0 ;; esac; } ;;
        good)     gh() { return 0; } ;;
      esac
      verify_attestation "$WORK/atlasctl-x86_64-unknown-linux-musl.tar.xz" ) 2>&1
}

contains "no gh at all: an invitation, not a warning" "$(attest absent)" "install \`gh\`"
out=$(attest old)
contains "gh too old: says so, and names the version" "$out" "too old"
case "$out" in *"could NOT be verified"*) bad "gh too old must not warn" "$out" ;; *) ok "gh too old must not warn" ;; esac

out=$(attest loggedout)
contains "gh signed out: says so" "$out" "not signed in"
case "$out" in *"could NOT be verified"*) bad "gh signed out must not warn" "$out" ;; *) ok "gh signed out must not warn" ;; esac

out=$(attest offline)
contains "unreachable GitHub: says so" "$out" "could not reach GitHub"
case "$out" in *"could NOT be verified"*) bad "an unreachable GitHub must not warn" "$out" ;; *) ok "an unreachable GitHub must not warn" ;; esac

contains "a capable gh that refuses IS a warning" "$(attest broken)" "could NOT be verified"
contains "a capable gh that verifies says so"     "$(attest good)"   "provenance verified"

# --- check_docker -------------------------------------------------------------
# Three distinct states, each with a different next action. Reporting any two of
# them the same way is how "install docker" gets said to someone who has it.
docker_state() { # absent | stopped | nogpu | fine
    ( . "$WORK/lib.sh"
      # shellcheck disable=SC2317  # called indirectly, by check_docker
      case "$1" in
        absent)  command() { if [ "$2" = docker ]; then return 1; fi; return 0; } ;;
        stopped) docker() { return 1; } ;;
        nogpu)   docker() { echo "Server Version: 29.1.3"; return 0; } ;;
        fine)    docker() { echo "Runtimes: nvidia runc"; return 0; } ;;
      esac
      # shellcheck disable=SC2317
      uname() { echo Linux; }
      check_docker ) 2>&1
}

contains "docker absent: says install, and what still works" \
    "$(docker_state absent)" "docker was not found"
out=$(docker_state stopped)
contains "docker installed but stopped: says START, not install" "$out" "did not answer"
case "$out" in *"was not found"*) bad "a stopped docker must not be called missing" "$out" ;; *) ok "a stopped docker must not be called missing" ;; esac
contains "no nvidia runtime: named separately" "$(docker_state nogpu)" "NVIDIA container runtime"
check "a healthy docker says nothing" "" "$(docker_state fine)"

# --- install_agent ------------------------------------------------------------
cat > "$WORK/fake-atlasctl" <<'EOF'
#!/bin/sh
case "$1" in
  --version) echo "atlasctl 9.9.9" ;;
  agent) case "$2" in
      status)  [ -n "${RUNNING:-}" ] && exit 0 || exit 1 ;;
      install) echo "[fake] agent install ran" ;;
    esac ;;
esac
EOF
chmod +x "$WORK/fake-atlasctl"

agent_run() { # same_version join running supervised
    ( . "$WORK/lib.sh"
      # shellcheck disable=SC2317  # both stubs are called by install_agent
      if [ "$4" = yes ]; then service_installed() { return 0; }; else service_installed() { return 1; }; fi
      RUNNING="$3" install_agent "$WORK/fake-atlasctl" "$2" "" "$1" ) 2>&1
}

contains "a fresh install installs the service" \
    "$(agent_run '' '' '' no)" "[fake] agent install ran"

contains "same version, nothing running: it STARTS what is there" \
    "$(agent_run yes '' '' yes)" "[fake] agent install ran"

contains "same version, running AND supervised: nothing to do" \
    "$(agent_run yes '' 1 yes)" "already running as a service"

# The regression that reached the user's symptom by another door: an agent
# started by hand answers the port with NO service behind it, and skipping on
# the port alone left the machine with nothing that survives a logout.
contains "answering the port without a service still installs one" \
    "$(agent_run yes '' 1 no)" "[fake] agent install ran"

contains "a join runs even when everything is already up" \
    "$(agent_run yes '12345678@10.0.0.1' 1 yes)" "[fake] agent install ran"

# --- install_agent, when the service install FAILS ----------------------------
# The branch an operator only ever sees on a bad day, and the one that used to
# tell them to run the command that had just failed. It must name the likeliest
# cause instead, and the platform's real log.
agent_fail() { # port_held: yes|no
    ( . "$WORK/lib.sh"
      # shellcheck disable=SC2317  # all called indirectly, by install_agent
      service_installed() { return 1; }
      # shellcheck disable=SC2317
      uname() { echo Linux; }
      if [ "$1" = yes ]; then
        # shellcheck disable=SC2317
        command() { case "$2" in lsof) return 0 ;; *) return 0 ;; esac; }
        # shellcheck disable=SC2317
        lsof() { return 0; }
      else
        # shellcheck disable=SC2317
        command() { case "$2" in lsof) return 1 ;; *) return 0 ;; esac; }
      fi
      install_agent "$WORK/failing-atlasctl" "" "" "" ) 2>&1
}
cat > "$WORK/failing-atlasctl" <<'EOF'
#!/bin/sh
case "$1 $2" in "agent install") exit 1 ;; esac
EOF
chmod +x "$WORK/failing-atlasctl"

out=$(agent_fail yes)
contains "a held port is named as the likely cause" "$out" "ALREADY listening"
# It must NOT then tell them to re-run the command that just failed — the dead
# end the macOS report opened with.
case "$out" in
    *"to see why the service install failed"*) bad "must not suggest re-running the failure" "$out" ;;
    *) ok "must not suggest re-running the failure" ;;
esac

out=$(agent_fail no)
contains "no held port: offers the foreground command" "$out" "agent run"
contains "and names the platform's real log"          "$out" "journalctl"

# --- option parsing -----------------------------------------------------------
# The REAL script, as a subprocess. Both of these decisions happen in main()
# before anything is fetched, so this cannot reach the network — and running the
# real thing is the only way to cover a parser that lives inside main().
# One invocation covers both: the unknown flag warns and parsing CONTINUES, so
# the `--join=` after it is still reached and still refuses.
parse_out=$(sh "$ROOT/scripts/install.sh" --bogus-flag --join= 2>&1); parse_rc=$?
contains "an unrecognised option is named, not silently swallowed" \
    "$parse_out" "ignoring unrecognised option: --bogus-flag"
contains "--join= with an empty value refuses" \
    "$parse_out" "--join= needs a value"
check "and refusing is non-zero, so a wrapper notices" "1" "$parse_rc"

printf '\n  %d passed, %d failed\n' "$pass" "$fail"
# Explicit, not inherited. `[ "$fail" -eq 0 ]` as the last command happens to
# be right here, but the PowerShell counterpart made exactly this mistake —
# reporting "0 failed" and then exiting 1, because a case above had run a child
# process that set the status.
if [ "$fail" -eq 0 ]; then exit 0; else exit 1; fi
