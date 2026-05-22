#!/bin/sh
set -eu

# Copy a SWAG/Let's Encrypt certificate lineage into a narrow directory for
# sorotte-server. Run this on the Docker host, typically as root.

if [ "$(id -u)" != "0" ]; then
    echo "error: run this script on the Docker host as root" >&2
    exit 1
fi

if [ -z "${SOROTTE_CERT_DOMAIN:-}" ]; then
    echo "error: set SOROTTE_CERT_DOMAIN, for example niceperson.club" >&2
    exit 1
fi

SWAG_ETC="${SWAG_ETC:-/mnt/user/appdata/swag/etc}"
SOURCE_DIR="${SWAG_CERT_DIR:-$SWAG_ETC/letsencrypt/live/$SOROTTE_CERT_DOMAIN}"
TARGET_DIR="${SOROTTE_TLS_DIR:-/mnt/user/appdata/sorotte-server/tls}"
TARGET_UID="${SOROTTE_UID:-10001}"
TARGET_GID="${SOROTTE_GID:-10001}"

for file in cert.pem chain.pem privkey.pem; do
    if [ ! -r "$SOURCE_DIR/$file" ]; then
        echo "error: cannot read $SOURCE_DIR/$file" >&2
        exit 1
    fi
done

umask 077
mkdir -p "$TARGET_DIR"
tmpdir="$(mktemp -d "$TARGET_DIR/.copy.XXXXXX")"

cleanup() {
    rm -rf "$tmpdir"
}
trap cleanup EXIT INT TERM

for file in cert.pem chain.pem privkey.pem; do
    cp "$SOURCE_DIR/$file" "$tmpdir/$file"
    chown "$TARGET_UID:$TARGET_GID" "$tmpdir/$file"
    chmod 0400 "$tmpdir/$file"
done

chown "$TARGET_UID:$TARGET_GID" "$tmpdir"
chmod 0500 "$tmpdir"

for file in cert.pem chain.pem privkey.pem; do
    mv -f "$tmpdir/$file" "$TARGET_DIR/$file"
done

chown "$TARGET_UID:$TARGET_GID" "$TARGET_DIR"
chmod 0500 "$TARGET_DIR"

echo "copied Sorotte TLS bundle from $SOURCE_DIR to $TARGET_DIR"
