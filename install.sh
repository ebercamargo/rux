#!/usr/bin/env bash
set -euo pipefail
REPO="ebercamargo/rux"
VERSION="${RUX_VERSION:-latest}"
INSTALL_DIR="${RUX_INSTALL_DIR:-$HOME/.local/bin}"
case "$(uname -s)-$(uname -m)" in
  Linux-x86_64) ASSET="rux-linux-x86_64.tar.gz" ;;
  Darwin-arm64) ASSET="rux-macos-aarch64.tar.gz" ;;
  *) echo "Unsupported OS/architecture" >&2; exit 1 ;;
esac
if [[ "$VERSION" == latest ]]; then API="https://api.github.com/repos/$REPO/releases/latest"
else API="https://api.github.com/repos/$REPO/releases/tags/$VERSION"; fi
command -v python3 >/dev/null || { echo "Python 3 required by installer" >&2; exit 1; }
readarray -t urls < <(python3 - "$API" "$ASSET" <<'PY'
import json,sys,urllib.request
request=urllib.request.Request(sys.argv[1],headers={"User-Agent":"rux-installer","Accept":"application/vnd.github+json"})
try:
    with urllib.request.urlopen(request) as response:
        data=json.load(response)
except Exception as e:
    raise SystemExit(f"No published RUX release available: {e}")
assets={a["name"]:a["browser_download_url"] for a in data["assets"]}
for name in (sys.argv[2],sys.argv[2]+".sha256"):
    if name not in assets: raise SystemExit(f"Missing release asset: {name}")
    print(assets[name])
PY
)
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
curl -fL --retry 3 "${urls[0]}" -o "$tmp/$ASSET"
curl -fL --retry 3 "${urls[1]}" -o "$tmp/$ASSET.sha256"
(cd "$tmp" && sha256sum -c "$ASSET.sha256")
tar -xzf "$tmp/$ASSET" -C "$tmp"
test -f "$tmp/rux"
mkdir -p "$INSTALL_DIR"
install -m 755 "$tmp/rux" "$INSTALL_DIR/rux"
"$INSTALL_DIR/rux" --version
printf 'RUX installed at %s/rux\nAdd %s to PATH if needed.\n' "$INSTALL_DIR" "$INSTALL_DIR"
