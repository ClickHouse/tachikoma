#!/usr/bin/env bash
# Forward the host's Docker daemon into a tachikoma VM.
#
# How it works:
#   1. Installs only the Docker CLI inside the VM (no engine/daemon)
#   2. Sets DOCKER_HOST to point at a socat TCP bridge on the host
#   3. The host bridge forwards TCP → Docker socket (Desktop or OrbStack)
#
# Usage:
#   # .tachikoma.toml
#   provision_scripts = ["./examples/docker-host-forwarding.sh"]
#
# You must also start the host-side bridge BEFORE spawning the VM:
#   ./examples/docker-bridge-start.sh
#
# ┌──────────────────────┐          ┌──────────────────────────────┐
# │ VM (Linux)           │          │ HOST (macOS)                 │
# │                      │          │                              │
# │ docker CLI           │──TCP────▶│ socat :2375                  │
# │ DOCKER_HOST=         │          │   ↓                          │
# │ tcp://192.168.64.1:  │          │ /var/run/docker.sock         │
# │ 2375                 │◀─────────│   ↓                          │
# │                      │          │ Docker Desktop / OrbStack    │
# └──────────────────────┘          └──────────────────────────────┘
#
# ⚠️  SECURITY WARNING:
#   Docker socket access grants effective root on the host machine.
#   Any process in the VM can:
#     - Mount host filesystems (docker run -v /:/host)
#     - Access host network stack
#     - Read Docker registry credentials
#     - Start unlimited containers
#   Only use this on trusted, single-user dev machines.

set -euo pipefail

DOCKER_HOST_IP="${DOCKER_HOST_IP:-192.168.64.1}"
DOCKER_HOST_PORT="${DOCKER_HOST_PORT:-2375}"

echo "[docker-forwarding] Installing Docker CLI..."
sudo apt-get update -qq
sudo apt-get install -y -qq docker-ce-cli 2>/dev/null || {
    # Add Docker's official GPG key and repo if docker-ce-cli isn't available
    sudo apt-get install -y -qq ca-certificates curl gnupg
    sudo install -m 0755 -d /etc/apt/keyrings
    curl -fsSL https://download.docker.com/linux/ubuntu/gpg | sudo gpg --dearmor -o /etc/apt/keyrings/docker.gpg
    sudo chmod a+r /etc/apt/keyrings/docker.gpg
    echo "deb [arch=$(dpkg --print-architecture) signed-by=/etc/apt/keyrings/docker.gpg] https://download.docker.com/linux/ubuntu $(. /etc/os-release && echo "$VERSION_CODENAME") stable" | \
        sudo tee /etc/apt/sources.list.d/docker.list > /dev/null
    sudo apt-get update -qq
    sudo apt-get install -y -qq docker-ce-cli
}

echo "[docker-forwarding] Configuring DOCKER_HOST → tcp://${DOCKER_HOST_IP}:${DOCKER_HOST_PORT}"
echo "export DOCKER_HOST=tcp://${DOCKER_HOST_IP}:${DOCKER_HOST_PORT}" >> ~/.profile

echo "[docker-forwarding] Verifying connection..."
if DOCKER_HOST="tcp://${DOCKER_HOST_IP}:${DOCKER_HOST_PORT}" docker info --format '{{.ServerVersion}}' 2>/dev/null; then
    echo "[docker-forwarding] Connected to host Docker daemon."
else
    echo "[docker-forwarding] WARNING: Could not reach Docker daemon at ${DOCKER_HOST_IP}:${DOCKER_HOST_PORT}."
    echo "[docker-forwarding] Make sure docker-bridge-start.sh is running on the host."
fi
