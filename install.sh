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

# Find the binary — tarball may contain a subdirectory
EXTRACTED="$(find "${TMPDIR}" -name "${BINARY_NAME}" -type f | head -1)"
if [ -z "${EXTRACTED}" ]; then
  echo "Archive did not contain ${BINARY_NAME}" >&2
  exit 1
fi
EXTRACTED_DIR="$(dirname "${EXTRACTED}")"

# Install the binary and any optional sibling tools present in the archive
mkdir -p "${INSTALL_DIR}"
for f in "${EXTRACTED_DIR}"/*; do
  mv "${f}" "${INSTALL_DIR}/$(basename "${f}")"
  chmod +x "${INSTALL_DIR}/$(basename "${f}")"
done

echo "Installed to ${INSTALL_DIR}/"

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

# Auto-configure agent MCP servers
BINARY_PATH="${INSTALL_DIR}/${BINARY_NAME}"

if command -v claude >/dev/null 2>&1; then
  echo ""
  claude mcp add court-jester -- "${BINARY_PATH}" 2>/dev/null \
    && echo "Configured Claude Code" \
    || echo "Claude Code: could not auto-configure. Run: claude mcp add court-jester -- ${BINARY_PATH}"
fi

if command -v codex >/dev/null 2>&1; then
  codex mcp add court-jester -- "${BINARY_PATH}" 2>/dev/null \
    && echo "Configured Codex CLI" \
    || echo "Codex CLI: could not auto-configure. Run: codex mcp add court-jester -- ${BINARY_PATH}"
fi

echo ""
echo "Done. Add this to your agent prompt:"
echo ""
echo "  After every code change, call court-jester verify on each changed file."
echo "  If verify returns overall_ok: false, fix the failing repro and verify again."
