#!/bin/sh
set -eu

# Install public prebuilt bpstr/codanna releases; default to latest stable.
REPO="bpstr/codanna"
INSTALL_DIR="${CODANNA_INSTALL_DIR:-$HOME/.local/bin}"
tmpdir=""
stage=""

say() { printf 'codanna: %s\n' "$*"; }
err() { printf 'codanna: ERROR: %s\n' "$*" >&2; exit 1; }
cleanup() {
    [ -z "$tmpdir" ] || rm -rf "$tmpdir"
    [ -z "$stage" ] || rm -rf "$stage"
}
trap cleanup 0
trap 'exit 1' 1 2 15

detect_platform() {
    case "$(uname -s)" in
        Linux) os="linux" ;;
        Darwin) os="macos" ;;
        *) err "supported systems are Linux and macOS" ;;
    esac
    case "$(uname -m)" in
        x86_64|amd64) arch="x64" ;;
        aarch64|arm64) arch="arm64" ;;
        *) err "unsupported architecture: $(uname -m)" ;;
    esac
    printf '%s-%s\n' "$os" "$arch"
}

main() {
    command -v curl >/dev/null 2>&1 || err "curl is required"
    command -v tar >/dev/null 2>&1 || err "tar with xz support is required"
    if command -v sha256sum >/dev/null 2>&1; then
        checksum_tool="sha256sum"
    elif command -v shasum >/dev/null 2>&1; then
        checksum_tool="shasum"
    else
        err "sha256sum or shasum is required"
    fi
    platform=$(detect_platform)
    tmpdir=$(mktemp -d)
    version="${CODANNA_VERSION:-}"
    if [ -z "$version" ]; then
        # GitHub's /latest endpoint excludes drafts and prereleases.
        status=$(curl -sSL --proto '=https' --proto-redir '=https' --tlsv1.2 \
            -o "$tmpdir/release.json" -w '%{http_code}' \
            "https://api.github.com/repos/$REPO/releases/latest") \
            || err "cannot contact GitHub to discover the latest stable release"
        case "$status" in
            200) ;;
            404) err "no published stable release is available for $REPO; publish a stable release or set CODANNA_VERSION to a published tag" ;;
            403|429) err "GitHub API request was denied or rate limited (HTTP $status); retry later or set CODANNA_VERSION to a published tag" ;;
            *) err "latest stable release lookup failed (HTTP $status)" ;;
        esac
        version=$(sed -n 's/^[[:space:]]*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$tmpdir/release.json")
    fi
    [ -n "$version" ] || err "GitHub response is missing a release tag"
    case "$version" in
        *[!a-zA-Z0-9.+_-]*) err "invalid release version: $version" ;;
    esac
    case "$version" in v*) ;; *) version="v$version" ;; esac
    directory="codanna-${version#v}-$platform"
    filename="$directory.tar.xz"

    say "downloading $REPO $version ($platform)"
    release_url="https://github.com/$REPO/releases/download/$version"
    curl -fsSL --proto '=https' --proto-redir '=https' --tlsv1.2 \
        "$release_url/$filename" -o "$tmpdir/$filename" \
        || err "download failed; check that $version is published and has a $platform binary"
    curl -fsSL --proto '=https' --proto-redir '=https' --tlsv1.2 \
        "$release_url/$filename.sha256" -o "$tmpdir/$filename.sha256" \
        || err "download failed; release checksum is unavailable"
    [ -f "$tmpdir/$filename" ] && [ -f "$tmpdir/$filename.sha256" ] \
        || err "release must contain both $filename and its .sha256 checksum"

    expected=$(awk 'NR == 1 { print $1 }' "$tmpdir/$filename.sha256")
    [ "${#expected}" -eq 64 ] || err "invalid SHA-256 checksum"
    case "$expected" in *[!0-9a-fA-F]*) err "invalid SHA-256 checksum" ;; esac
    if [ "$checksum_tool" = "sha256sum" ]; then
        actual=$(sha256sum "$tmpdir/$filename")
    else
        actual=$(shasum -a 256 "$tmpdir/$filename")
    fi
    actual=${actual%% *}
    expected=$(printf '%s' "$expected" | tr 'A-F' 'a-f')
    [ "$actual" = "$expected" ] || err "checksum mismatch; existing installation was preserved"

    # Extract only the expected executable from the release archive.
    tar -xJf "$tmpdir/$filename" -C "$tmpdir" "$directory/codanna" \
        || err "release archive does not contain $directory/codanna"
    binary="$tmpdir/$directory/codanna"
    [ -f "$binary" ] && [ ! -L "$binary" ] || err "release executable is not a regular file"
    mkdir -p "$INSTALL_DIR"
    [ ! -d "$INSTALL_DIR/codanna" ] || err "install destination is a directory: $INSTALL_DIR/codanna"
    stage=$(mktemp -d "$INSTALL_DIR/.codanna-install.XXXXXX")
    cp "$binary" "$stage/codanna"
    chmod 755 "$stage/codanna"
    mv -f "$stage/codanna" "$INSTALL_DIR/codanna"
    say "installed $version to $INSTALL_DIR/codanna"
    case ":$PATH:" in
        *":$INSTALL_DIR:"*) ;;
        *) say "add to your shell profile: export PATH=\"$INSTALL_DIR:\$PATH\"" ;;
    esac
    say "run $INSTALL_DIR/codanna --version to verify it on this machine"
}

main "$@"
