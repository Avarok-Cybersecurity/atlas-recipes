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
        *)
            die "unsupported operating system: $os. atlasctl supports Linux and macOS; Windows is not supported."
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

    expected=$(grep "  $name\$" "$sums" | awk '{print $1}' || true)
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
    if command -v gh >/dev/null 2>&1; then
        if gh attestation verify "$archive" --repo "$REPO" >/dev/null 2>&1; then
            info "build provenance verified"
        else
            warn "build provenance could NOT be verified for $(basename "$archive")."
            warn "If you did not expect that, stop and check the release page."
        fi
    else
        info "install \`gh\` to also verify build provenance: gh attestation verify <file> --repo $REPO"
    fi
}

# Put the agent behind this machine's own supervisor, so the website has
# something to talk to after a reboot rather than only while a terminal is open.
#
# Never fatal. A container with no user systemd bus is a normal place to install
# atlasctl, and the CLI works fully without an agent — so a failure here is a
# note, not an error. ATLASCTL_NO_AGENT=1 skips it for anyone who would rather
# decide when a background process with docker access starts running.
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
    info "installing the background agent"
    if "$exe" agent install >/dev/null 2>&1; then
        info "the agent is running and will start on login."
        info "pair your browser with: $BIN_NAME agent token"
        return
    fi
    warn "could not install the agent as a service on this machine."
    warn "atlasctl itself is installed and works. To run the agent by hand:"
    warn "    $BIN_NAME agent run"
    warn "or, to see why the service install failed:"
    warn "    $BIN_NAME agent install"
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
    # Install to a temp name and rename, so an interrupted install cannot leave
    # a half-written binary on PATH.
    install -m 0755 "$tmp/$BIN_NAME" "$dir/.$BIN_NAME.new"
    mv -f "$dir/.$BIN_NAME.new" "$dir/$BIN_NAME"
    info "installed $dir/$BIN_NAME"

    case ":$PATH:" in
        *":$dir:"*) ;;
        *)
            warn "$dir is not on your PATH. Add it, e.g.:"
            warn "    echo 'export PATH=\"\$PATH:$dir\"' >> ~/.profile"
            ;;
    esac

    check_docker
    check_sparkrun
    install_agent "$dir/$BIN_NAME"

    info "done. Try:"
    info "    $BIN_NAME list"
    info "    $BIN_NAME run qwen3.6-35b-a3b-fp8-mtp"
}

main "$@"
