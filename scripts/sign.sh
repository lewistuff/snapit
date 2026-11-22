#!/bin/bash
# Signs the app bundle with a stable identity so macOS keeps the Screen
# Recording grant across rebuilds (TCC keys the grant to the designated
# requirement, which for ad-hoc signatures changes on every build).
#
# With SIGN_ID set, signs with that identity from the default keychains.
# Otherwise creates (once) a self-signed "snapit-dev" certificate in its own
# keychain, separate from the login keychain, and signs with it.
set -euo pipefail
APP=$1
KC="$HOME/Library/Keychains/snapit-dev.keychain-db"
NAME=snapit-dev

if [ -n "${SIGN_ID:-}" ]; then
  exec codesign --force --sign "$SIGN_ID" --options runtime "$APP"
fi

if [ ! -f "$KC" ]; then
  echo "creating self-signed '$NAME' code-signing certificate in $KC"
  TMP=$(mktemp -d)
  trap 'rm -rf "$TMP"' EXIT
  # macOS's own LibreSSL writes PKCS#12 files `security` can import; newer
  # OpenSSL builds default to ciphers it can't read.
  OPENSSL=/usr/bin/openssl
  $OPENSSL req -x509 -newkey rsa:2048 -nodes -days 3650 -subj "/CN=$NAME" \
    -keyout "$TMP/key.pem" -out "$TMP/cert.pem" \
    -addext "keyUsage=critical,digitalSignature" \
    -addext "extendedKeyUsage=critical,codeSigning" \
    -addext "basicConstraints=critical,CA:false" 2>/dev/null
  $OPENSSL pkcs12 -export -inkey "$TMP/key.pem" -in "$TMP/cert.pem" \
    -out "$TMP/id.p12" -passout pass:snapit
  security create-keychain -p "" "$KC"
  if ! { security set-keychain-settings "$KC" && # never auto-lock
         security unlock-keychain -p "" "$KC" &&
         security import "$TMP/id.p12" -k "$KC" -P snapit -T /usr/bin/codesign >/dev/null &&
         security set-key-partition-list -S apple-tool:,apple: -s -k "" "$KC" >/dev/null; }; then
    security delete-keychain "$KC"
    exit 1
  fi
fi

security unlock-keychain -p "" "$KC"
codesign --force --sign "$NAME" --keychain "$KC" --options runtime "$APP"
