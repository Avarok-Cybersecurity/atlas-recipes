#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-only
#
# Atlas launcher installer — https://atlasinference.io/install.sh
#
#   curl -fsSL https://atlasinference.io/install.sh | sh
#
# Everything is wrapped in main() and called on the last line, so a download
# that is cut off part-way executes nothing at all rather than half of an
# installer.
#
# This script deliberately embeds no version. It always resolves
# releases/latest, which is what keeps a cached copy on the website correct
# even when it is older than the release it installs.

set -eu

REPO="Avarok-Cybersecurity/atlas-recipes"
BIN_NAME="atlasctl"

info() { printf '\033[1;36m[atlas]\033[0m %s\n' "$1" >&2; }
warn() { printf '\033[1;33m[atlas]\033[0m %s\n' "$1" >&2; }
err()  { printf '\033[1;31m[atlas]\033[0m %s\n' "$1" >&2; }

die() { err "$1"; exit 1; }

need() {
    command -v "$1" >/dev/null 2>&1 || die "$1 is required but was not found on PATH."
}

detect_target() {
    os=$(uname -s)
    arch=$(uname -m)
    case "$os" in
        Linux)  os_part="unknown-linux-musl" ;;
        Darwin) os_part="apple-darwin" ;;
        # Git Bash and MSYS reach here on Windows. They are not the way in:
        # the Windows build is native, and the installer for it is a different
        # script. Naming that beats "not supported", which was true when it was
        # written and would now send someone away from a working install.
        MINGW*|MSYS*|CYGWIN*|Windows_NT)
            die "this is the unix installer. On Windows, run in PowerShell:
    irm https://atlasinference.io/install.ps1 | iex"
            ;;
        *)
            die "unsupported operating system: $os. atlasctl supports Linux, macOS and Windows."
            ;;
    esac
    case "$arch" in
        aarch64|arm64) arch_part="aarch64" ;;
        x86_64|amd64)  arch_part="x86_64" ;;
        *)
            die "unsupported architecture: $arch. atlasctl supports aarch64 and x86_64."
            ;;
    esac
    printf '%s-%s' "$arch_part" "$os_part"
}

# Verify a downloaded archive against the release's SHA256SUMS.
verify_checksum() {
    archive="$1"
    sums="$2"
    name=$(basename "$archive")

    # An exact field comparison, not a regex match. `grep "  $name\$"` put the
    # filename into a pattern, where `.` matches any character — so
    # `atlasctl-x86_64-unknown-linux-musl.tar.xz` also matched a line naming
    # `...-muslXtarXxz`. Nothing generates such a name today, which is the whole
    # problem with leaving it: the check that decides whether to execute a
    # downloaded binary should not depend on that staying true.
    #
    # The `*` form is accepted because `sha256sum -b` writes it, and a SHA256SUMS
    # produced that way is not malformed.
    # Every match, so a SUMS naming the same file twice with DIFFERENT hashes is
    # refused rather than resolved. Taking the first (or the last, as the
    # PowerShell installer did) means a stale entry beside a current one decides
    # silently which bytes are acceptable.
    # Lowercased before de-duplicating. PowerShell's `-ne` is case-INsensitive,
    # so a SUMS carrying the same hash twice in different letter case was an
    # "identical duplicate, verifies" on Windows and a "different hashes,
    # refuse" here — one release, two verdicts.
    expected=$(awk -v n="$name" '$2 == n || $2 == "*" n { print tolower($1) }' "$sums" | sort -u)
    case "$expected" in
        *"
"*) die "SHA256SUMS lists $name more than once, with different hashes. Refusing to install." ;;
    esac
    [ -n "$expected" ] || die "no checksum for $name in SHA256SUMS; refusing to install."

    if command -v sha256sum >/dev/null 2>&1; then
        actual=$(sha256sum "$archive" | awk '{print $1}')
    elif command -v shasum >/dev/null 2>&1; then
        actual=$(shasum -a 256 "$archive" | awk '{print $1}')
    else
        die "neither sha256sum nor shasum is available; cannot verify the download."
    fi

    [ "$expected" = "$actual" ] || die "checksum mismatch for $name. Refusing to install."
    info "checksum verified"
}

# Sigstore provenance, when the user happens to have gh. Not required: making
# the one-liner depend on GitHub's CLI would defeat the point of a one-liner.
verify_attestation() {
    archive="$1"
    if ! command -v gh >/dev/null 2>&1; then
        info "install \`gh\` to also verify build provenance: gh attestation verify <file> --repo $REPO"
        return
    fi
    # "Cannot check" and "checked, and it does not verify" are different facts
    # and only the second is alarming. They were reported identically, so every
    # machine with a gh older than 2.49 — which has no `attestation` subcommand
    # at all — saw "build provenance could NOT be verified. If you did not
    # expect that, stop and check the release page." on a perfectly good
    # install. A security warning that fires on the healthy case is worse than
    # none: it teaches the operator to scroll past the real one.
    if ! gh attestation --help >/dev/null 2>&1; then
        info "this \`gh\` is too old to check build provenance (needs 2.49+)."
        info "The checksum above was verified; provenance is the extra step."
        return
    fi
    # The check is ATTEMPTED, then its failure is classified. Deciding in
    # advance on `gh auth status` skipped it entirely for a gh that is installed
    # and never logged in — which is the state every machine is in right after
    # `apt install gh` — so a tampered archive served with a matching
    # SHA256SUMS would have installed in silence. Attempting first also catches
    # the case a pre-check cannot see at all: gh signed in, GitHub unreachable.
    if err=$(gh attestation verify "$archive" --repo "$REPO" 2>&1); then
        info "build provenance verified"
        return
    fi
    # It failed. Whether that is a fact about the ARTIFACT or about this machine
    # decides whether anyone should be alarmed.
    case "$err" in
        *"not logged"*|*"authentication"*|*"gh auth login"*|*"HTTP 401"*|*"HTTP 403"*)
            info "\`gh\` is not signed in, so provenance could not be checked."
            info "The checksum above was verified; provenance is the extra step."
            info "    gh auth login && gh attestation verify <file> --repo $REPO"
            ;;
        *"dial tcp"*|*"no such host"*|*"connection refused"*|*"i/o timeout"*|*"TLS handshake"*|*"context deadline"*)
            info "could not reach GitHub, so provenance was not checked."
            info "The checksum above was verified; provenance is the extra step."
            ;;
        *)
            # gh could ask, reached GitHub, and the answer was no.
            warn "build provenance could NOT be verified for $(basename "$archive")."
            warn "If you did not expect that, stop and check the release page."
            ;;
    esac
}

# Put the agent behind this machine's own supervisor, so the website has
# something to talk to after a reboot rather than only while a terminal is open.
#
# Never fatal. A container with no user systemd bus is a normal place to install
# atlasctl, and the CLI works fully without an agent — so a failure here is a
# note, not an error. ATLASCTL_NO_AGENT=1 skips it for anyone who would rather
# decide when a background process with docker access starts running.
# Whether a SUPERVISOR knows about the agent, as opposed to a process happening
# to hold the port. The two are different machines to come back to tomorrow.
service_installed() {
    case "$(uname -s)" in
        Darwin) launchctl print "gui/$(id -u)/io.atlasinference.atlasctl-agent" >/dev/null 2>&1 ;;
        Linux)  systemctl --user is-enabled atlasctl-agent.service >/dev/null 2>&1 ;;
        *)      return 1 ;;
    esac
}

install_agent() {
    # The binary's path is passed rather than read from a variable another
    # function happened to set: install.sh runs under `set -eu` piped from the
    # network, and an unset global there fails at the least helpful moment.
    exe="$1"
    if [ -n "${ATLASCTL_NO_AGENT:-}" ]; then
        info "skipping the background agent (ATLASCTL_NO_AGENT is set)."
        info "start it yourself later with: $BIN_NAME agent install"
        return
    fi
    # Re-running the installer on a machine that already has this exact version
    # is the commonest reason to run it at all: the binary is there, the service
    # is not up, and the website says "no agent". Nothing needs installing --
    # the machine needs STARTING. A join is the exception: it is the step the
    # operator came for, so it runs regardless.
    if [ -n "${4:-}" ] && [ -z "${2:-}" ]; then
        # BOTH conditions, not just the port. An operator who ran `atlasctl
        # agent run` by hand has an agent answering on 34333 and NO service, and
        # skipping on the port alone left it that way: the terminal closes, the
        # box reboots, and "the website says no agent" is back — the very
        # symptom this script was changed to fix, reached through another door.
        if "$exe" agent status >/dev/null 2>&1 && service_installed; then
            info "the agent is already running as a service — this machine is ready."
            info "if a browser does not see it, pair it: $BIN_NAME agent token"
            return
        fi
        info "starting the agent that is already installed here"
    else
        info "installing the background agent"
    fi
    # The join is NOT silenced: it is the step the operator is watching, and
    # its output carries the verification words they are meant to compare.
    if [ -n "${2:-}" ]; then
        if "$exe" agent install --join "$2" ${3:+"$3"}; then
            return
        fi
        # One exit code covers two steps. Saying which failed would need the
        # installer to report them separately, so say plainly that either could
        # have, and give the check that distinguishes them — rather than
        # asserting the install worked and only the join did not, which sent
        # operators to mint a fresh code for a service that was never created.
        warn "the agent install, the fleet join, or both did not succeed."
        warn "Check which with:  $BIN_NAME agent status"
        warn "A join code is single-use and expires. To retry just the join:"
        warn "    $BIN_NAME agent install --join <code>@<host>"
        return
    fi
    # NOT silenced. `agent install` deliberately reports two things the
    # operator can only act on if they see them: that enable-linger failed, so
    # the agent will stop at logout on a headless box, and that the unit was
    # accepted but the agent is not actually running.
    if "$exe" agent install; then
        info "pair your browser with: $BIN_NAME agent token"
        return
    fi
    warn "could not install the agent as a service on this machine."
    warn "atlasctl itself is installed and works."
    # "Run agent install to see why" was the old advice, and it reproduced the
    # SAME line the operator had just read — a third dead end after the failed
    # bootstrap. Check the likeliest cause here instead: an agent already
    # holding the port is what makes a re-install look like a broken install.
    if command -v lsof >/dev/null 2>&1 && lsof -nP -iTCP:34333 -sTCP:LISTEN >/dev/null 2>&1; then
        warn "an agent is ALREADY listening on 127.0.0.1:34333, so this machine"
        warn "is usable right now — the website will find it. Nothing else to do."
        warn "To point that agent at this newly installed binary, stop it and run:"
        warn "    $BIN_NAME agent install"
        return
    fi
    warn "To run the agent by hand:"
    warn "    $BIN_NAME agent run"
    warn "Its startup log, if the service did start and then exit:"
    if [ "$(uname -s)" = "Darwin" ]; then
        warn "    tail -n 50 ~/Library/Logs/atlasctl-agent.log"
    else
        warn "    journalctl --user -u atlasctl-agent -n 50"
    fi
}

check_docker() {
    if ! command -v docker >/dev/null 2>&1; then
        warn "docker was not found. \`atlasctl run\` needs it; \`atlasctl list\` and"
        warn "\`atlasctl run --print\` work fine without it."
        return
    fi
    if ! docker info >/dev/null 2>&1; then
        warn "docker is installed but its daemon did not answer. Start it before \`atlasctl run\`."
        return
    fi
    if [ "$(uname -s)" = "Linux" ] && ! docker info 2>/dev/null | grep -qi nvidia; then
        warn "the NVIDIA container runtime was not detected. GPU recipes need it."
    fi
}

# Report a sparkrun install whose registry has been redirected.
#
# We never touch the user's files. The tool being replaced was compromised by
# an upstream that quietly rewrote configuration; an installer that quietly
# deletes things is not the answer to that.
check_sparkrun() {
    cfg="${HOME}/.config/sparkrun/registries.yaml"
    found=""
    command -v sparkrun >/dev/null 2>&1 && found="yes"
    [ -f "$cfg" ] && grep -q "Atlas-Inf/sparkrun-recipes" "$cfg" 2>/dev/null && found="redirected"

    [ -n "$found" ] || return 0

    warn ""
    warn "================ SECURITY NOTICE ================"
    warn "A sparkrun install was found on this machine."
    if [ "$found" = "redirected" ]; then
        warn "Its config points the \`atlas\` registry at Atlas-Inf/sparkrun-recipes,"
        warn "which Atlas does not control, and marks it trusted. A trusted registry's"
        warn "recipes can run shell commands on this host."
        warn "Editing that file is not enough — the redirect is compiled into sparkrun"
        warn "and is reapplied the next time it runs."
    fi
    warn "To remove it:"
    warn "    pipx uninstall sparkrun     # or: uv tool uninstall sparkrun"
    warn "    rm -rf ~/.config/sparkrun ~/.cache/sparkrun"
    warn "Review those directories first. This installer will not delete them for you."
    warn "================================================="
    warn ""
}

do_uninstall() {
    dir="${ATLASCTL_INSTALL_DIR:-$HOME/.local/bin}"
    if [ -f "$dir/$BIN_NAME" ]; then
        # The service MUST come out before the binary does. `main` installs the
        # agent service by default, and its unit carries Restart=on-failure with
        # RestartSec=5. Deleting the binary underneath a live unit leaves
        # systemd relaunching a path that no longer exists, every five seconds,
        # forever — a failed unit and journal spam that reinstalling elsewhere
        # does not clear, because the stale unit is still the one enabled.
        # Best-effort: an uninstall must finish even if this half cannot.
        if "$dir/$BIN_NAME" agent uninstall >/dev/null 2>&1; then
            info "removed the agent service"
        else
            info "no agent service to remove (or it could not be reached)"
        fi
        rm -f "$dir/$BIN_NAME"
        info "removed $dir/$BIN_NAME"
    else
        info "nothing to remove in $dir"
    fi
    for d in "$HOME/.config/atlasctl" "$HOME/.cache/atlasctl"; do
        [ -d "$d" ] && info "left in place: $d"
    done
    exit 0
}

main() {
    [ "${1:-}" = "--uninstall" ] && do_uninstall

    # `--join <code>@<host>` is forwarded verbatim to `agent install`, which is
    # the only thing that understands it. Parsed here only far enough to notice
    # it was given without a value, because the alternative is installing and
    # then failing on the step the operator actually came for.
    join=""
    grant_control=""
    while [ $# -gt 0 ]; do
        case "$1" in
            --join)
                shift
                [ $# -gt 0 ] || die "--join needs a value, e.g. --join 12345678@10.10.10.1"
                join="$1"
                ;;
            --join=*)
                join="${1#--join=}"
                # `--join=` with nothing after it read as "no join requested", so
                # the machine installed, never joined, and the operator walked
                # away believing it had. install.ps1 refuses this; so does this.
                [ -n "$join" ] || die "--join= needs a value, e.g. --join=12345678@10.10.10.1"
                ;;
            # Forwarded, not interpreted: it means "let the fleet I am joining
            # run models on this machine", and only `agent install` can act on
            # it. Noticed here so the operator is told what they just consented
            # to, on the machine they consented on.
            --grant-control) grant_control="--grant-control" ;;
            # Silence here is how a misspelling (`--grant-contol`, `-Join`)
            # becomes an install that quietly did not do what was asked.
            # Warn rather than die: the one-liner is pasted by hand, and
            # refusing outright would strand someone over a typo they can see.
            --*) warn "ignoring unrecognised option: $1" ;;
            *) ;;
        esac
        shift
    done
    [ -z "$join" ] || info "will join the fleet at ${join#*@} once installed"
    [ -z "$grant_control" ] || info "and will let that fleet run models on this machine"

    need uname
    need tar
    if command -v curl >/dev/null 2>&1; then
        fetch() { curl -fsSL "$1" -o "$2"; }
    elif command -v wget >/dev/null 2>&1; then
        fetch() { wget -qO "$2" "$1"; }
    else
        die "neither curl nor wget is available."
    fi

    target=$(detect_target)
    version="${ATLASCTL_VERSION:-latest}"
    if [ "$version" = "latest" ]; then
        base="https://github.com/$REPO/releases/latest/download"
    else
        base="https://github.com/$REPO/releases/download/$version"
    fi

    tmp=$(mktemp -d)
    # shellcheck disable=SC2064  # expand $tmp now, not at trap time
    trap "rm -rf '$tmp'" EXIT INT TERM

    archive_name="${BIN_NAME}-${target}.tar.xz"
    info "downloading $archive_name"
    fetch "$base/$archive_name" "$tmp/$archive_name" \
        || die "could not download $base/$archive_name — is there a release for $target yet?"
    fetch "$base/SHA256SUMS" "$tmp/SHA256SUMS" \
        || die "could not download SHA256SUMS; refusing to install unverified binaries."

    verify_checksum "$tmp/$archive_name" "$tmp/SHA256SUMS"
    verify_attestation "$tmp/$archive_name"

    tar -xf "$tmp/$archive_name" -C "$tmp"

    dir="${ATLASCTL_INSTALL_DIR:-$HOME/.local/bin}"
    mkdir -p "$dir"
    [ -f "$tmp/$BIN_NAME" ] || die "the archive did not contain $BIN_NAME"

    # Ask the two binaries their versions rather than parsing a release tag: the
    # downloaded one is already here and already checksum-verified, so this
    # needs no second endpoint and cannot be fooled by a tag naming scheme that
    # changes. An unreadable or non-answering existing binary compares unequal,
    # which lands on the upgrade path -- the safe direction.
    new_version=$("$tmp/$BIN_NAME" --version 2>/dev/null || true)
    old_version=""
    [ -x "$dir/$BIN_NAME" ] && old_version=$("$dir/$BIN_NAME" --version 2>/dev/null || true)

    same_version=""
    if [ -n "$new_version" ] && [ "$old_version" = "$new_version" ]; then
        same_version="yes"
        info "$new_version is already installed here — keeping it."
    else
        [ -z "$old_version" ] || info "upgrading $old_version -> $new_version"
        # Install to a temp name and rename, so an interrupted install cannot leave
        # a half-written binary on PATH.
        install -m 0755 "$tmp/$BIN_NAME" "$dir/.$BIN_NAME.new"
        mv -f "$dir/.$BIN_NAME.new" "$dir/$BIN_NAME"
        info "installed $dir/$BIN_NAME"
    fi

    case ":$PATH:" in
        *":$dir:"*) ;;
        *)
            warn "$dir is not on your PATH. Add it, e.g.:"
            warn "    echo 'export PATH=\"\$PATH:$dir\"' >> ~/.profile"
            ;;
    esac

    check_docker
    check_sparkrun
    install_agent "$dir/$BIN_NAME" "$join" "$grant_control" "$same_version"

    info "done. Try:"
    info "    $BIN_NAME list"
    info "    $BIN_NAME run qwen3.6-35b-a3b-fp8-mtp"
}

main "$@"
