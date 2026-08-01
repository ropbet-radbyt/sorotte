#!/bin/sh
set -eu

# Publish a SWAG/Let's Encrypt certificate lineage as an immutable Sorotte TLS
# generation, then atomically switch current.json. Run this on the Docker host,
# typically as root.

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
GENERATIONS_DIR="$TARGET_DIR/generations"

for file in cert.pem chain.pem privkey.pem; do
    if [ ! -r "$SOURCE_DIR/$file" ]; then
        echo "error: cannot read $SOURCE_DIR/$file" >&2
        exit 1
    fi
done

if ! command -v readlink >/dev/null 2>&1; then
    echo "error: readlink is required to resolve the immutable Let's Encrypt lineage" >&2
    exit 1
fi
if ! command -v sha256sum >/dev/null 2>&1; then
    echo "error: sha256sum is required to publish an authenticated TLS manifest" >&2
    exit 1
fi

resolve_lineage_member() {
    lineage_filename="$1"
    lineage_prefix="$2"
    resolved_lineage_path="$(readlink -f "$SOURCE_DIR/$lineage_filename")" || {
        echo "error: cannot resolve $SOURCE_DIR/$lineage_filename" >&2
        exit 1
    }
    if [ ! -r "$resolved_lineage_path" ]; then
        echo "error: resolved lineage member is not readable: $resolved_lineage_path" >&2
        exit 1
    fi
    resolved_lineage_basename="${resolved_lineage_path##*/}"
    case "$resolved_lineage_basename" in
        "$lineage_prefix"[0-9]*.pem)
            resolved_lineage_serial="${resolved_lineage_basename#"$lineage_prefix"}"
            resolved_lineage_serial="${resolved_lineage_serial%.pem}"
            ;;
        *)
            echo "error: $lineage_filename must resolve to immutable ${lineage_prefix}<number>.pem, got $resolved_lineage_basename" >&2
            exit 1
            ;;
    esac
    case "$resolved_lineage_serial" in
        "" | *[!0-9]*)
            echo "error: $lineage_filename has an invalid Let's Encrypt lineage number" >&2
            exit 1
            ;;
    esac
    resolved_lineage_directory="${resolved_lineage_path%/*}"
}

resolve_lineage_member cert.pem cert
cert_source="$resolved_lineage_path"
lineage_serial="$resolved_lineage_serial"
lineage_directory="$resolved_lineage_directory"
resolve_lineage_member chain.pem chain
chain_source="$resolved_lineage_path"
if [ "$resolved_lineage_serial" != "$lineage_serial" ] ||
    [ "$resolved_lineage_directory" != "$lineage_directory" ]; then
    echo "error: SWAG lineage changed while resolving cert.pem and chain.pem" >&2
    exit 1
fi
resolve_lineage_member privkey.pem privkey
privkey_source="$resolved_lineage_path"
if [ "$resolved_lineage_serial" != "$lineage_serial" ] ||
    [ "$resolved_lineage_directory" != "$lineage_directory" ]; then
    echo "error: SWAG lineage changed while resolving cert.pem and privkey.pem" >&2
    exit 1
fi

umask 077
mkdir -p "$TARGET_DIR"
if [ ! -d "$TARGET_DIR" ] || [ -L "$TARGET_DIR" ]; then
    echo "error: Sorotte TLS root must be a plain directory: $TARGET_DIR" >&2
    exit 1
fi
mkdir -p "$GENERATIONS_DIR"
if [ ! -d "$GENERATIONS_DIR" ] || [ -L "$GENERATIONS_DIR" ]; then
    echo "error: Sorotte generations root must be a plain directory: $GENERATIONS_DIR" >&2
    exit 1
fi
staging_dir="$(mktemp -d "$GENERATIONS_DIR/.staging.XXXXXX")"
manifest_tmp=""

cleanup() {
    if [ -n "${staging_dir:-}" ] && [ -d "$staging_dir" ]; then
        rm -rf -- "$staging_dir"
    fi
    if [ -n "${manifest_tmp:-}" ] && [ -f "$manifest_tmp" ]; then
        rm -f -- "$manifest_tmp"
    fi
}
trap cleanup EXIT INT TERM

for file in cert.pem chain.pem privkey.pem; do
    case "$file" in
        cert.pem) source_path="$cert_source" ;;
        chain.pem) source_path="$chain_source" ;;
        privkey.pem) source_path="$privkey_source" ;;
    esac
    cp "$source_path" "$staging_dir/$file"
    chown "$TARGET_UID:$TARGET_GID" "$staging_dir/$file"
    chmod 0400 "$staging_dir/$file"
done

read_manifest_member() {
    member_path="$staging_dir/$1"
    member_length="$(wc -c < "$member_path" | tr -d '[:space:]')"
    member_digest="$(sha256sum "$member_path")"
    member_digest="${member_digest%% *}"
    if [ "${#member_digest}" -ne 64 ]; then
        echo "error: sha256sum returned an invalid digest for $member_path" >&2
        exit 1
    fi
    case "$member_digest" in
        *[!0-9a-f]*)
            echo "error: sha256sum returned a non-canonical digest for $member_path" >&2
            exit 1
            ;;
    esac
}

read_manifest_member privkey.pem
privkey_length="$member_length"
privkey_digest="$member_digest"
read_manifest_member cert.pem
cert_length="$member_length"
cert_digest="$member_digest"
read_manifest_member chain.pem
chain_length="$member_length"
chain_digest="$member_digest"

for file in cert.pem chain.pem privkey.pem; do
    case "$file" in
        cert.pem)
            source_path="$cert_source"
            expected_digest="$cert_digest"
            ;;
        chain.pem)
            source_path="$chain_source"
            expected_digest="$chain_digest"
            ;;
        privkey.pem)
            source_path="$privkey_source"
            expected_digest="$privkey_digest"
            ;;
    esac
    source_digest="$(sha256sum "$source_path")"
    source_digest="${source_digest%% *}"
    if [ "$source_digest" != "$expected_digest" ]; then
        echo "error: immutable SWAG lineage member changed during capture: $source_path" >&2
        exit 1
    fi
done

staging_suffix="${staging_dir##*.staging.}"
generation_id="$(date -u +%Y%m%dT%H%M%SZ)-le$lineage_serial-$$-$staging_suffix"
case "$generation_id" in
    [A-Za-z0-9]*[A-Za-z0-9]) ;;
    *)
        echo "error: generated unsafe TLS generation ID: $generation_id" >&2
        exit 1
        ;;
esac
case "$generation_id" in
    *[!A-Za-z0-9_-]*)
        echo "error: generated unsafe TLS generation ID: $generation_id" >&2
        exit 1
        ;;
esac
if [ "${#generation_id}" -gt 128 ]; then
    echo "error: generated TLS generation ID is longer than 128 bytes" >&2
    exit 1
fi

generation_dir="$GENERATIONS_DIR/$generation_id"
if [ -e "$generation_dir" ]; then
    echo "error: TLS generation already exists: $generation_dir" >&2
    exit 1
fi

chown "$TARGET_UID:$TARGET_GID" "$staging_dir"
chmod 0500 "$staging_dir"
mv "$staging_dir" "$generation_dir"
staging_dir=""

chown "$TARGET_UID:$TARGET_GID" "$GENERATIONS_DIR"
chmod 0500 "$GENERATIONS_DIR"
chown "$TARGET_UID:$TARGET_GID" "$TARGET_DIR"
chmod 0500 "$TARGET_DIR"

# Make the complete immutable generation durable before publishing its selector.
sync

manifest_tmp="$(mktemp "$TARGET_DIR/.current.XXXXXX")"
{
    printf '{\n'
    printf '  "schema": "sorotte-tls-bundle-v1",\n'
    printf '  "generation": "%s",\n' "$generation_id"
    printf '  "members": {\n'
    printf '    "privkey.pem": {"length": %s, "sha256": "%s"},\n' \
        "$privkey_length" "$privkey_digest"
    printf '    "cert.pem": {"length": %s, "sha256": "%s"},\n' \
        "$cert_length" "$cert_digest"
    printf '    "chain.pem": {"length": %s, "sha256": "%s"}\n' \
        "$chain_length" "$chain_digest"
    printf '  }\n'
    printf '}\n'
} > "$manifest_tmp"
chown "$TARGET_UID:$TARGET_GID" "$manifest_tmp"
chmod 0400 "$manifest_tmp"
mv -f "$manifest_tmp" "$TARGET_DIR/current.json"
manifest_tmp=""

# Retain older immutable generations: readers that observed the previous
# selector may still be using them. Their certificate-sized footprint is small
# and they can be garbage-collected only with an operator-controlled grace
# period.
sync

echo "published Sorotte TLS generation $generation_id from $SOURCE_DIR to $TARGET_DIR"
