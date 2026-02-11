#!/bin/sh
set -eu

REPO="vibesrc/coop"

main() {
    arch=$(uname -m)
    case "$arch" in
        x86_64|amd64)  target="x86_64-unknown-linux-gnu" ;;
        aarch64|arm64) target="aarch64-unknown-linux-gnu" ;;
        *)
            echo "Error: unsupported architecture: $arch" >&2
            echo "coop only supports x86_64 and aarch64 Linux." >&2
            exit 1
            ;;
    esac

    os=$(uname -s)
    if [ "$os" != "Linux" ]; then
        echo "Error: unsupported OS: $os" >&2
        echo "coop only supports Linux." >&2
        exit 1
    fi

    echo "Detecting latest release..."
    tag=$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" \
        | grep '"tag_name"' | head -1 | cut -d'"' -f4)

    if [ -z "$tag" ]; then
        echo "Error: could not determine latest release." >&2
        exit 1
    fi

    echo "Latest release: $tag"

    url="https://github.com/${REPO}/releases/download/${tag}/coop-${target}.tar.gz"

    if [ "$(id -u)" -eq 0 ]; then
        install_dir="/usr/local/bin"
    else
        install_dir="${HOME}/.local/bin"
        mkdir -p "$install_dir"
    fi

    tmpdir=$(mktemp -d)
    trap 'rm -rf "$tmpdir"' EXIT

    echo "Downloading coop for ${target}..."
    curl -fsSL "$url" -o "${tmpdir}/coop.tar.gz"

    echo "Installing to ${install_dir}/coop..."
    tar xzf "${tmpdir}/coop.tar.gz" -C "$tmpdir"
    install -m 755 "${tmpdir}/coop" "${install_dir}/coop"

    if "${install_dir}/coop" --version >/dev/null 2>&1; then
        echo "Installed $("${install_dir}/coop" --version)"
    else
        echo "Installed coop to ${install_dir}/coop"
    fi

    # Check if install_dir is in PATH
    case ":$PATH:" in
        *":${install_dir}:"*) ;;
        *)
            echo ""
            echo "NOTE: ${install_dir} is not in your PATH."
            echo "Add it with:  export PATH=\"${install_dir}:\$PATH\""
            ;;
    esac
}

main
