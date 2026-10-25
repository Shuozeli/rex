#!/bin/bash
# Starts the recording desktop: Xvfb, openbox, and Google Chrome (stable) with the extension installed the way
# an organization deploys it. At start, /ext is packed into a CRX with /keys/ext.pem; a local update server
# serves it, and the ExtensionSettings policy force-installs and pins it. Chrome runs in a loop, so
# `pkill -TERM -f /opt/google/chrome/chrome` gives a fresh browser on the same profile.
#
# Env: BIND (default 127.0.0.1; compose passes the Tailscale IP), CDP_PORT (9541), VNC_PORT (5941),
#      NOVNC_PORT (6081), SCREEN (1920x1080), CHROME_ARGS (extra flags, e.g. --proxy-server=socks5://host:port).
# Hermetic mode (REX_HERMETIC="<scenario.json> [...]"): every host resolves to 127.0.0.1, where fixture-server.mjs
#      serves the scenario's fixture pages and the bundle in /bundle over TLS from a CA made here and trusted
#      in the profile's NSS store. Nothing leaves the container.
# With arguments, the desktop starts in the background, the arguments run as a command once Chrome answers,
# and the container exits with the command's status (`docker compose run e2e`).
set -euo pipefail

BIND=${BIND:-127.0.0.1}
CDP_PORT=${CDP_PORT:-9541}
VNC_PORT=${VNC_PORT:-5941}
NOVNC_PORT=${NOVNC_PORT:-6081}
SCREEN=${SCREEN:-1920x1080}
export DISPLAY=:99
PROFILE=/home/rec/profile
KEY=/keys/ext.pem
PACK=/tmp/rex-pack
EXT_PORT=8099

Xvfb :99 -screen 0 "${SCREEN}x24" -nolisten tcp -ac &
for _ in $(seq 50); do xdpyinfo >/dev/null 2>&1 && break; sleep 0.1; done
openbox &
xsetroot -cursor_name left_ptr -solid '#202124'

# Pack the built extension into a CRX (Chrome's own packer), signed with the e2e key.
[ -r "$KEY" ] || { echo "no signing key at $KEY (mount ~/.config/rex/ext-e2e-key.pem)"; exit 1; }
rm -rf "$PACK" && mkdir -p "$PACK" && cp -r /ext "$PACK/rex"
google-chrome --pack-extension="$PACK/rex" --pack-extension-key="$KEY" --user-data-dir=/tmp/pack-profile \
  --no-first-run >/dev/null 2>&1 || true
[ -s "$PACK/rex.crx" ] || { echo "packing the extension failed"; exit 1; }

# The id follows from the key: sha256 of the public key (DER), first 32 hex digits mapped 0-f -> a-p.
EXT_ID=$(openssl pkey -in "$KEY" -pubout -outform DER 2>/dev/null | sha256sum | cut -c1-32 | tr 0-9a-f a-p)
EXT_VERSION=$(node -e 'console.log(JSON.parse(require("fs").readFileSync("/ext/manifest.json","utf8")).version)')
echo "$EXT_ID" > /tmp/rex-ext-id
echo "extension $EXT_ID $EXT_VERSION ($(stat -c %s "$PACK/rex.crx") bytes CRX)"

cat > /etc/opt/chrome/policies/managed/rex-extension.json <<JSON
{"ExtensionSettings": {"$EXT_ID": {
  "installation_mode": "force_installed",
  "update_url": "http://127.0.0.1:$EXT_PORT/updates.xml",
  "toolbar_pin": "force_pinned"}}}
JSON
node /usr/local/lib/rex/ext-server.mjs "$PACK/rex.crx" "$EXT_ID" "$EXT_VERSION" "$EXT_PORT" &

HERMETIC_ARGS=()
if [ -n "${REX_HERMETIC:-}" ]; then
  TLS=/tmp/rex-tls && mkdir -p "$TLS"
  FIXTURE_SERVER=/repo/extension/recorder/docker/fixture-server.mjs
  mapfile -t HOSTS < <(node "$FIXTURE_SERVER" --hosts "$REX_HERMETIC" /repo)
  SAN=$(printf 'DNS:%s,' "${HOSTS[@]}"); SAN=${SAN%,}
  openssl req -x509 -newkey rsa:2048 -nodes -days 2 -subj "/CN=rex e2e CA" -keyout "$TLS/ca.key" -out "$TLS/ca.pem" \
    -addext basicConstraints=critical,CA:TRUE -addext keyUsage=critical,keyCertSign,cRLSign 2>/dev/null
  openssl req -newkey rsa:2048 -nodes -subj "/CN=rex e2e" -keyout "$TLS/site.key" -out "$TLS/site.csr" 2>/dev/null
  printf 'subjectAltName=%s\nextendedKeyUsage=serverAuth\n' "$SAN" > "$TLS/ext.cnf"
  openssl x509 -req -in "$TLS/site.csr" -CA "$TLS/ca.pem" -CAkey "$TLS/ca.key" -CAcreateserial -days 2 \
    -extfile "$TLS/ext.cnf" -out "$TLS/site.pem" 2>/dev/null
  mkdir -p /home/rec/.pki/nssdb
  certutil -d sql:/home/rec/.pki/nssdb -N --empty-password 2>/dev/null || true
  certutil -d sql:/home/rec/.pki/nssdb -A -n "rex e2e CA" -t "C,," -i "$TLS/ca.pem"
  node "$FIXTURE_SERVER" "$REX_HERMETIC" /repo /bundle "$TLS/site.pem" "$TLS/site.key" &
  HERMETIC_ARGS=(--host-resolver-rules="MAP * 127.0.0.1, EXCLUDE localhost")
  echo "hermetic: ${#HOSTS[@]} hosts -> 127.0.0.1"
fi

node /usr/local/lib/rex/cdp-relay.mjs "$BIND" "$CDP_PORT" &
x11vnc -display :99 -rfbport "$VNC_PORT" -listen 127.0.0.1 -nopw -forever -shared -quiet &
websockify --web /usr/share/novnc "${BIND}:${NOVNC_PORT}" "127.0.0.1:${VNC_PORT}" >/dev/null 2>&1 &
echo "CDP http://${BIND}:${CDP_PORT}  noVNC http://${BIND}:${NOVNC_PORT}/vnc.html"

mkdir -p "$PROFILE"
W=${SCREEN%x*}
H=${SCREEN#*x}
# Chrome's own logging goes to a file (hermetic runs are full of expected TLS errors for Google's hosts).
CHROME_LOG=${CHROME_LOG:-/tmp/chrome.log}
chrome_loop() {
while true; do
  google-chrome \
    --user-data-dir="$PROFILE" \
    --remote-debugging-port=9222 \
    --no-first-run --no-default-browser-check \
    --window-position=0,0 --window-size="$W,$H" --start-maximized \
    --disable-dev-shm-usage --hide-crash-restore-bubble \
    "${HERMETIC_ARGS[@]}" ${CHROME_ARGS:-} \
    about:blank >>"$CHROME_LOG" 2>&1 || true
  echo "chrome exited; restarting"
  sleep 1
done
}

if [ $# -eq 0 ]; then
  chrome_loop
fi
chrome_loop &
for _ in $(seq 120); do curl -sf http://127.0.0.1:9222/json/version >/dev/null && break; sleep 0.5; done
"$@"
