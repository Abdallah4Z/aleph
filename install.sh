#!/usr/bin/env bash
set -euo pipefail

REPO="Abdallah4Z/aleph"
VERSION="${ALEPH_VERSION:-latest}"
BIN_DIR="${HOME}/.local/bin"
DATA_DIR="${HOME}/.local/share/aleph"
CONFIG_DIR="${HOME}/.config/aleph"
SERVICE_DIR="${HOME}/.config/systemd/user"

GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
NC='\033[0m'
BOLD='\033[1m'

log()  { echo -e "  ${GREEN}✓${NC} $1"; }
info() { echo -e "  ${BLUE}→${NC} $1"; }
warn() { echo -e "  ${YELLOW}⚠${NC} $1"; }
fail() { echo -e "  ${RED}✗${NC} $1"; exit 1; }
header() {
  echo ""
  echo -e "${BOLD}$1${NC}"
  echo -e "${BOLD}$(printf '═%0.s' $(seq 1 ${#1}))${NC}"
}

header "Aleph — Installer"
echo ""

# --- Pre-flight checks ---
info "Checking system requirements..."
command -v curl &>/dev/null || fail "curl is required. Install it first."
command -v tar &>/dev/null  || fail "tar is required."
command -v systemctl &>/dev/null || fail "systemd is required. Aleph installs as a systemd user service."
echo ""

# --- Detect display ---
DISPLAY_VAL="${DISPLAY:-:0}"

# --- Detect distro and install system deps ---
header "1. System Dependencies"
case "$(uname -s)" in
  Linux)
    if command -v apt-get &>/dev/null; then
      info "Detected apt-based distro (Debian/Ubuntu)"
      sudo apt-get update -qq || warn "apt update failed, continuing..."
      sudo apt-get install -y -qq libxcb1-dev libdbus-1-dev libxdo-dev libx11-dev protobuf-compiler || \
        fail "Failed to install system dependencies. Try: sudo apt-get install libxcb1-dev libdbus-1-dev libxdo-dev libx11-dev protobuf-compiler"
      log "System dependencies installed"
    elif command -v pacman &>/dev/null; then
      info "Detected Arch-based distro"
      sudo pacman -S --noconfirm libxcb dbus libxdo libx11 protobuf || \
        fail "Failed to install system dependencies. Try: sudo pacman -S libxcb dbus libxdo libx11 protobuf"
      log "System dependencies installed"
    elif command -v dnf &>/dev/null; then
      info "Detected Fedora-based distro"
      sudo dnf install -y libxcb-devel dbus-devel libxdo-devel libX11-devel protobuf-compiler || \
        fail "Failed to install system dependencies. Try: sudo dnf install libxcb-devel dbus-devel libxdo-devel libX11-devel protobuf-compiler"
      log "System dependencies installed"
    else
      warn "Unknown package manager. You may need to install build deps manually."
    fi
    ;;
  *)
    fail "Unsupported OS: $(uname -s). Aleph requires Linux."
    ;;
esac
echo ""

# --- Download prebuilt binary ---
header "2. Aleph Binary"

mkdir -p "${BIN_DIR}"

if [ "${VERSION}" = "latest" ]; then
  DOWNLOAD_URL="https://github.com/${REPO}/releases/latest/download/aleph-x86_64-linux.tar.gz"
  info "Fetching latest release..."
else
  DOWNLOAD_URL="https://github.com/${REPO}/releases/download/${VERSION}/aleph-x86_64-linux.tar.gz"
  info "Fetching release ${VERSION}..."
fi

# Follow redirects and show progress
curl -#fSL "${DOWNLOAD_URL}" -o /tmp/aleph.tar.gz || fail "Failed to download Aleph binary from ${DOWNLOAD_URL}"
log "Downloaded Aleph tarball ($(du -h /tmp/aleph.tar.gz | cut -f1))"

tar xzf /tmp/aleph.tar.gz -C /tmp/ || fail "Failed to extract tarball"
cp /tmp/aleph-x86_64-linux/aleph "${BIN_DIR}/aleph"
chmod +x "${BIN_DIR}/aleph"
rm -f /tmp/aleph.tar.gz
log "Installed to ${BIN_DIR}/aleph ($(du -h ${BIN_DIR}/aleph | cut -f1))"
echo ""

# --- Download model weights ---
header "3. ML Model Weights"

mkdir -p "${DATA_DIR}/models"

download_model() {
  local dir="$1"; shift
  local base_url="$1"; shift
  local total=$#
  local count=0
  mkdir -p "${DATA_DIR}/models/${dir}"
  for file in "$@"; do
    count=$((count + 1))
    local dest="${DATA_DIR}/models/${dir}/${file}"
    if [ -f "$dest" ] && [ -s "$dest" ]; then
      info "  [${count}/${total}] ${file} — already exists, skipping"
    else
      info "  [${count}/${total}] ${file} — downloading..."
      curl -#fSL "${base_url}/${file}" -o "${dest}" || \
        warn "Failed to download ${file}. You can re-run the script to retry."
      log "  ${file} saved ($(du -h "$dest" | cut -f1))"
    fi
  done
}

info "MiniLM text encoder (87 MB)..."
download_model "all-MiniLM-L6-v2" \
  "https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2/resolve/main" \
  "config.json" "tokenizer.json" "model.safetensors"

info "SigLIP vision encoder (775 MB)..."
download_model "siglip" \
  "https://huggingface.co/google/siglip-base-patch16-224/resolve/main" \
  "config.json" "model.safetensors"

log "All model weights downloaded"
echo ""

# --- Create default config ---
header "4. Configuration"

mkdir -p "${CONFIG_DIR}"
cat > "${CONFIG_DIR}/config.toml" << CONF
[general]
data_dir = "${DATA_DIR}"
port = 2198
log_level = "info"

[polling]
interval_secs = 2

[dedup]
threshold = 0.95
last_n = 5

[encoders]
text = true
vision = true

[retention]
max_events = 10000

[dashboard]
theme = "dark"
CONF
log "Config written to ${CONFIG_DIR}/config.toml"
echo ""

# --- Enable AT-SPI accessibility ---
header "5. Accessibility"
if command -v gsettings &>/dev/null; then
  gsettings set org.gnome.desktop.interface toolkit-accessibility true 2>/dev/null && \
    log "AT-SPI accessibility enabled" || \
    warn "Could not enable AT-SPI (GNOME not detected). Aleph will use xcap for screenshots."
else
  info "gsettings not available — Aleph will use xcap screenshot capture"
fi
echo ""

# --- Install systemd service ---
header "6. Systemd Service"

mkdir -p "${SERVICE_DIR}"
cat > "${SERVICE_DIR}/aleph.service" << SERVICE
[Unit]
Description=Aleph — Context Store
After=graphical-session.target

[Service]
Type=simple
ExecStart=${BIN_DIR}/aleph start
Restart=on-failure
RestartSec=3
Environment=DISPLAY=${DISPLAY_VAL}

[Install]
WantedBy=default.target
SERVICE
log "Service file written to ${SERVICE_DIR}/aleph.service"
echo ""

# --- Ensure PATH includes ~/.local/bin ---
header "7. Shell Setup"
if ! echo "${PATH}" | tr ':' '\n' | grep -q "${BIN_DIR}"; then
  export PATH="${BIN_DIR}:${PATH}"
  for rc in "${HOME}/.bashrc" "${HOME}/.zshrc"; do
    if [ -f "$rc" ]; then
      if ! grep -q 'aleph' "$rc" 2>/dev/null; then
        echo "export PATH=\"\${HOME}/.local/bin:\${PATH}\"" >> "$rc"
      fi
    fi
  done
  log "Added ${BIN_DIR} to PATH in .bashrc / .zshrc"
else
  info "${BIN_DIR} already in PATH"
fi
echo ""

# --- Auto-start: the final breath ---
header "8. Launching Aleph"

systemctl --user daemon-reload || fail "systemctl daemon-reload failed"
systemctl --user enable --now aleph || fail "Failed to start Aleph via systemd. Check: systemctl --user status aleph"

log "Aleph is running!"
echo ""
info "  Dashboard: ${BOLD}http://localhost:2198${NC}"
info "  Settings:  ${BOLD}http://localhost:2198/settings${NC}"
info "  Config:    ${BOLD}${CONFIG_DIR}/config.toml${NC}"
info "  Data:      ${BOLD}${DATA_DIR}${NC}"
echo ""
echo -e "  ${BOLD}Aleph is already capturing your screen context.${NC}"
echo -e "  ${BOLD}Open the dashboard to see it in action.${NC}"
echo ""
