#!/usr/bin/env bash
set -euo pipefail

ROOT=""
REPO="xoo0524tw/Tenacity-Launcher"
ASSET_NAME="Tenacity.jar"

while [[ $# -gt 0 ]]; do
    case "$1" in
        --root)  ROOT="$2";  shift 2 ;;
        --repo)  REPO="$2";  shift 2 ;;
        --asset) ASSET_NAME="$2"; shift 2 ;;
        *)       echo "Unknown argument: $1"; exit 1 ;;
    esac
done

if [ -z "$ROOT" ]; then
    echo "[Updater] Error: --root is required."
    exit 1
fi

ROOT="${ROOT%/}/"

SAVE_DIR="${ROOT}save"
JAR_PATH="${ROOT}${ASSET_NAME}"
VERSION_FILE="${SAVE_DIR}Tenacity.version"
TEMP_PATH="${SAVE_DIR}${ASSET_NAME}.download"

mkdir -p "$SAVE_DIR"

DOWNLOADER=""
if command -v curl &>/dev/null; then
    DOWNLOADER="curl"
elif command -v wget &>/dev/null; then
    DOWNLOADER="wget"
else
    echo "[Updater] Error: curl or wget is required."
    exit 1
fi

echo "[Updater] Checking latest Tenacity release..."

RELEASE_JSON=""
if [ "$DOWNLOADER" = "curl" ]; then
    RELEASE_JSON="$(curl -fsSL \
        -H "User-Agent: Tenacity-Launcher" \
        -H "Accept: application/vnd.github+json" \
        "https://api.github.com/repos/${REPO}/releases/latest" 2>&1)" || {
        echo "[Updater] Error fetching release info from GitHub."
        exit 1
    }
else
    RELEASE_JSON="$(wget -qO- \
        --header="User-Agent: Tenacity-Launcher" \
        --header="Accept: application/vnd.github+json" \
        "https://api.github.com/repos/${REPO}/releases/latest" 2>&1)" || {
        echo "[Updater] Error fetching release info from GitHub."
        exit 1
    }
fi

LATEST_TAG=""
if command -v python3 &>/dev/null; then
    LATEST_TAG="$(echo "$RELEASE_JSON" | python3 -c "import sys,json; print(json.load(sys.stdin).get('tag_name',''))")"
elif command -v python &>/dev/null; then
    LATEST_TAG="$(echo "$RELEASE_JSON" | python -c "import sys,json; print(json.load(sys.stdin).get('tag_name',''))")"
else
    LATEST_TAG="$(echo "$RELEASE_JSON" | grep -o '"tag_name"\s*:\s*"[^"]*"' | head -1 | sed 's/.*"tag_name"\s*:\s*"\([^"]*\)".*/\1/')"
fi

if [ -z "$LATEST_TAG" ]; then
    echo "[Updater] Error: Could not parse tag name from GitHub response."
    exit 1
fi

DOWNLOAD_URL=""
if command -v python3 &>/dev/null; then
    DOWNLOAD_URL="$(echo "$RELEASE_JSON" | python3 -c "
import sys, json
data = json.load(sys.stdin)
for a in data.get('assets', []):
    if a.get('name') == '${ASSET_NAME}':
        print(a.get('browser_download_url',''))
        break
else:
    for a in data.get('assets', []):
        if a.get('name','').endswith('.jar') and 'Tenacity' in a.get('name',''):
            print(a.get('browser_download_url',''))
            break
")"
elif command -v python &>/dev/null; then
    DOWNLOAD_URL="$(echo "$RELEASE_JSON" | python -c "
import sys, json
data = json.load(sys.stdin)
for a in data.get('assets', []):
    if a.get('name') == '${ASSET_NAME}':
        print(a.get('browser_download_url',''))
        break
else:
    for a in data.get('assets', []):
        if a.get('name','').endswith('.jar') and 'Tenacity' in a.get('name',''):
            print(a.get('browser_download_url',''))
            break
")"
else
    DOWNLOAD_URL="$(echo "$RELEASE_JSON" | grep -o '"browser_download_url"\s*:\s*"[^"]*\.jar"' | head -1 | sed 's/.*"\(https:.*\)".*/\1/')"
fi

if [ -z "$DOWNLOAD_URL" ]; then
    echo "[Updater] Error: Release $LATEST_TAG does not contain ${ASSET_NAME}."
    exit 1
fi

LOCAL_TAG=""
if [ -f "$VERSION_FILE" ]; then
    LOCAL_TAG="$(cat "$VERSION_FILE" | tr -d '[:space:]')"
fi

if [ -f "$JAR_PATH" ] && [ "$LOCAL_TAG" = "$LATEST_TAG" ]; then
    echo "[Updater] Tenacity.jar is up to date ($LATEST_TAG)."
    exit 0
fi

if [ -f "$JAR_PATH" ]; then
    echo "[Updater] Updating Tenacity.jar: $LOCAL_TAG -> $LATEST_TAG"
else
    echo "[Updater] Downloading Tenacity.jar ($LATEST_TAG)"
fi

rm -f "$TEMP_PATH"

if [ "$DOWNLOADER" = "curl" ]; then
    curl -fSL \
        -H "User-Agent: Tenacity-Launcher" \
        -o "$TEMP_PATH" \
        "$DOWNLOAD_URL" || {
        echo "[Updater] Download failed."
        rm -f "$TEMP_PATH"
        exit 1
    }
else
    wget --header="User-Agent: Tenacity-Launcher" \
        -O "$TEMP_PATH" \
        "$DOWNLOAD_URL" || {
        echo "[Updater] Download failed."
        rm -f "$TEMP_PATH"
        exit 1
    }
fi

FILE_SIZE=$(stat -c%s "$TEMP_PATH" 2>/dev/null || stat -f%z "$TEMP_PATH" 2>/dev/null || echo "0")
if [ "$FILE_SIZE" -lt 1048576 ]; then
    echo "[Updater] Error: Downloaded Tenacity.jar is unexpectedly small (${FILE_SIZE} bytes)."
    rm -f "$TEMP_PATH"
    exit 1
fi

mv -f "$TEMP_PATH" "$JAR_PATH"
echo "$LATEST_TAG" > "$VERSION_FILE"

echo "[Updater] Tenacity.jar is ready ($LATEST_TAG)."
