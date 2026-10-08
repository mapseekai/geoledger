#!/usr/bin/env bash
# Create a private CA plus gateway and PostGIS certificates for
# deploy/compose.production.yaml. For evaluation and drills only: production uses
# certificates from your PKI or ACME, with the same file names.
#   scripts/make-test-certs.sh <dir> [gateway names...]
set -euo pipefail
dir=${1:?usage: make-test-certs.sh <dir> [gateway DNS names...]}
shift
names=("$@")
((${#names[@]})) || names=(api.geoledger.example grpc.geoledger.example console.geoledger.example)
mkdir -p "$dir"
umask 077
openssl req -x509 -newkey rsa:3072 -nodes -days 90 -subj "/CN=GeoLedger test-only CA" \
  -keyout "$dir/ca.key" -out "$dir/ca.crt" 2>/dev/null
issue() { # name, SAN list
  openssl req -newkey rsa:2048 -nodes -subj "/CN=$1" -keyout "$dir/$1.key" -out "$dir/$1.csr" 2>/dev/null
  printf 'subjectAltName=%s\nextendedKeyUsage=serverAuth\n' "$2" > "$dir/$1.ext"
  openssl x509 -req -in "$dir/$1.csr" -CA "$dir/ca.crt" -CAkey "$dir/ca.key" -CAcreateserial \
    -days 90 -extfile "$dir/$1.ext" -out "$dir/$1.crt" 2>/dev/null
  rm "$dir/$1.csr" "$dir/$1.ext"
}
san=$(printf 'DNS:%s,' "${names[@]}")
issue gateway "${san%,}"
issue postgis "DNS:postgis"
# Certificates are public; keys stay 0600 (containers start as root and copy them).
chmod 0755 "$dir"
chmod 0644 "$dir"/*.crt
rm -f "$dir/ca.srl"
echo "wrote $dir/{ca,gateway,postgis}.{crt,key} (test-only CA, valid 90 days)"
