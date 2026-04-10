#!/bin/sh
set -e

REPO="slee1996/court-jester-mcp"
INSTALL_DIR="${HOME}/.local/bin"
BINARY_NAME="court-jester-mcp"

# Detect platform
OS="$(uname -s)"
ARCH="$(uname -m)"

case "${OS}" in
  Darwin) os="darwin" ;;
  Linux)  os="linux" ;;
  *)      echo "Unsupported OS: ${OS}" >&2; exit 1 ;;
esac

case "${ARCH}" in
  arm64|aarch64) arch="arm64" ;;
  x86_64|amd64)  arch="amd64" ;;
  *)             echo "Unsupported architecture: ${ARCH}" >&2; exit 1 ;;
esac

PLATFORM="${os}-${arch}"

# Get latest release tag
TAG="$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" | grep '"tag_name"' | head -1 | cut -d'"' -f4)"
if [ -z "${TAG}" ]; then
  echo "Failed to fetch latest release" >&2
  exit 1
fi

VERSION="${TAG#v}"
ASSET="court-jester-${TAG}-${PLATFORM}.tar.gz"
URL="https://github.com/${REPO}/releases/download/${TAG}/${ASSET}"

echo "Installing ${BINARY_NAME} ${TAG} (${PLATFORM})..."

# Download and extract
TMPDIR="$(mktemp -d)"
trap 'rm -rf "${TMPDIR}"' EXIT

if ! curl -fsSL "${URL}" -o "${TMPDIR}/${ASSET}"; then
  echo "No release binary for ${PLATFORM}. Available binaries at:" >&2
  echo "  https://github.com/${REPO}/releases/latest" >&2
  echo "" >&2
  echo "You can build from source instead:" >&2
  echo "  cargo install --git https://github.com/${REPO}.git" >&2
  exit 1
fi

tar xzf "${TMPDIR}/${ASSET}" -C "${TMPDIR}"

# Install
mkdir -p "${INSTALL_DIR}"
mv "${TMPDIR}/${BINARY_NAME}" "${INSTALL_DIR}/${BINARY_NAME}"
chmod +x "${INSTALL_DIR}/${BINARY_NAME}"

echo "Installed to ${INSTALL_DIR}/${BINARY_NAME}"

# Check PATH
case ":${PATH}:" in
  *":${INSTALL_DIR}:"*) ;;
  *)
    echo ""
    echo "Add ${INSTALL_DIR} to your PATH:"
    echo "  export PATH=\"${INSTALL_DIR}:\${PATH}\""
    echo ""
    echo "Add that line to your ~/.zshrc or ~/.bashrc to make it permanent."
    ;;
esac

echo ""
echo "Next: connect to your agent"
echo "  claude mcp add court-jester -- court-jester-mcp"
echo "  codex mcp add court-jester -- court-jester-mcp"
