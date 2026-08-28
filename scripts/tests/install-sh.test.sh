#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-only
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

sed 's/^main "\$@"$//' "$ROOT/scripts/install.sh" > "$WORK/lib.sh"

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

verify() { ( . "$WORK/lib.sh"; verify_checksum "$1" "$2" ) 2>&1; }

printf '%s  atlasctl-x86_64-unknown-linux-musl.tar.xz\n' "$good" > "$WORK/SUMS.good"
contains "a matching checksum is accepted" \
    "$(verify "$WORK/atlasctl-x86_64-unknown-linux-musl.tar.xz" "$WORK/SUMS.good")" "checksum verified"

printf '%s  atlasctl-x86_64-unknown-linux-musl.tar.xz\n' "${good%??}00" > "$WORK/SUMS.bad"
contains "a mismatched checksum refuses to install" \
    "$(verify "$WORK/atlasctl-x86_64-unknown-linux-musl.tar.xz" "$WORK/SUMS.bad")" "Refusing to install"

printf '%s  some-other-file.tar.xz\n' "$good" > "$WORK/SUMS.absent"
contains "an archive with no entry refuses to install" \
    "$(verify "$WORK/atlasctl-x86_64-unknown-linux-musl.tar.xz" "$WORK/SUMS.absent")" "no checksum for"

# The regression: the name was interpolated into a REGEX, so `.` matched any
# character and a line naming a DIFFERENT file satisfied the lookup — handing
# back a hash for bytes nobody checked.
printf '%s  atlasctl-x86_64-unknown-linux-muslXtarXxz\n' "$good" > "$WORK/SUMS.regex"
contains "a name that only matches as a regex is not accepted" \
    "$(verify "$WORK/atlasctl-x86_64-unknown-linux-musl.tar.xz" "$WORK/SUMS.regex")" "no checksum for"

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

printf '\n  %d passed, %d failed\n' "$pass" "$fail"
[ "$fail" -eq 0 ]
