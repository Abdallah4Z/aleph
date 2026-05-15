#!/usr/bin/env bash
set -euo pipefail

REPO="Abdallah4Z/aleph"
VERSION="${ALEPH_VERSION:-latest}"
BIN_DIR="${HOME}/.local/bin"
DATA_DIR="${HOME}/.local/share/aleph"
CONFIG_DIR="${HOME}/.config/aleph"
SERVICE_DIR="${HOME}/.config/systemd/user"

# --- Detect distro and install system deps ---
if command -v apt-get &>/dev/null; then
  sudo apt-get update -qq
  sudo apt-get install -y -qq libxcb1-dev libdbus-1-dev libxdo-dev libx11-dev protobuf-compiler 2>/dev/null
elif command -v pacman &>/dev/null; then
  sudo pacman -S --noconfirm libxcb dbus libxdo libx11 protobuf 2>/dev/null
elif command -v dnf &>/dev/null; then
  sudo dnf install -y libxcb-devel dbus-devel libxdo-devel libX11-devel protobuf-compiler 2>/dev/null
fi

# --- Download prebuilt binary ---
mkdir -p "${BIN_DIR}"
if [ "${VERSION}" = "latest" ]; then
  DOWNLOAD_URL="https://github.com/${REPO}/releases/latest/download/aleph-x86_64-linux.tar.gz"
else
  DOWNLOAD_URL="https://github.com/${REPO}/releases/download/${VERSION}/aleph-x86_64-linux.tar.gz"
fi

echo "Downloading Aleph binary..."
curl -fsSL "${DOWNLOAD_URL}" -o /tmp/aleph.tar.gz
tar xzf /tmp/aleph.tar.gz -C /tmp/
cp /tmp/aleph-x86_64-linux/aleph "${BIN_DIR}/aleph"
chmod +x "${BIN_DIR}/aleph"
rm -f /tmp/aleph.tar.gz

# --- Download model weights ---
mkdir -p "${DATA_DIR}/models"

download_model() {
  local dir="$1"; shift
  local base_url="$1"; shift
  mkdir -p "${DATA_DIR}/models/${dir}"
  for file in "$@"; do
    if [ ! -f "${DATA_DIR}/models/${dir}/${file}" ]; then
      echo "  Downloading ${file}..."
      curl -fsSL "${base_url}/${file}" -o "${DATA_DIR}/models/${dir}/${file}"
    fi
  done
}

echo "Downloading MiniLM text encoder..."
download_model "all-MiniLM-L6-v2" \
  "https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2/resolve/main" \
  "config.json" "tokenizer.json" "model.safetensors"

echo "Downloading SigLIP vision encoder..."
download_model "siglip" \
  "https://huggingface.co/google/siglip-base-patch16-224/resolve/main" \
  "config.json" "model.safetensors"

# --- Create default config ---
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

# --- Enable AT-SPI accessibility ---
gsettings set org.gnome.desktop.interface toolkit-accessibility true 2>/dev/null || true

# --- Install systemd service ---
mkdir -p "${SERVICE_DIR}"
cat > "${SERVICE_DIR}/aleph.service" << SERVICE
[Unit]
Description=Aleph — Context Store
After=graphical-session.target

[Service]
Type=simple
ExecStart=${BIN_DIR}/aleph
Restart=on-failure
RestartSec=3
Environment=DISPLAY=:0

[Install]
WantedBy=default.target
SERVICE

# --- Ensure PATH includes ~/.local/bin ---
if ! echo "${PATH}" | tr ':' '\n' | grep -q "${BIN_DIR}"; then
  if [ -f "${HOME}/.bashrc" ]; then
    echo "export PATH=\"\${HOME}/.local/bin:\${PATH}\"" >> "${HOME}/.bashrc"
  fi
  if [ -f "${HOME}/.zshrc" ]; then
    echo "export PATH=\"\${HOME}/.local/bin:\${PATH}\"" >> "${HOME}/.zshrc"
  fi
fi

# --- Auto-start: the final breath ---
systemctl --user daemon-reload
systemctl --user enable --now aleph
