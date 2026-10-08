#!/usr/bin/env bash
# Render deploy/gateway/nginx.conf against a disposable local server and verify TLS
# termination for the HTTP API and gRPC (via the gl CLI). Requires nginx, openssl, curl.
set -euo pipefail
cd "$(dirname "$0")/.."
server_bin=${GL_SERVER_BIN:-target/debug/geoledger-server}
cli_bin=${GL_CLI_BIN:-target/debug/gl}
nginx_bin=${NGINX:-$(command -v nginx || echo /usr/sbin/nginx)}
work=$(mktemp -d)
cleanup() {
  [[ -f "$work/nginx.pid" ]] && kill "$(cat "$work/nginx.pid")" 2>/dev/null || true
  [[ -n "${server_pid:-}" ]] && kill "$server_pid" 2>/dev/null || true
  rm -rf "$work"
}
trap cleanup EXIT
port() { python3 -c 'import socket;s=socket.socket();s.bind(("127.0.0.1",0));print(s.getsockname()[1])'; }
http_port=$(port); grpc_port=$(port); tls_port=$(port); plain_port=$(port)
# Test-only PKI: CA and a gateway certificate for localhost/127.0.0.1.
openssl req -x509 -newkey rsa:2048 -nodes -days 1 -subj "/CN=gateway test-only CA" \
  -keyout "$work/ca.key" -out "$work/ca.crt" 2>/dev/null
openssl req -newkey rsa:2048 -nodes -subj "/CN=localhost" -keyout "$work/gateway.key" \
  -out "$work/gateway.csr" 2>/dev/null
printf 'subjectAltName=DNS:localhost,IP:127.0.0.1\n' > "$work/san.ext"
openssl x509 -req -in "$work/gateway.csr" -CA "$work/ca.crt" -CAkey "$work/ca.key" \
  -CAcreateserial -days 1 -extfile "$work/san.ext" -out "$work/gateway.crt" 2>/dev/null
"$server_bin" --data-dir "$work/data" --http "127.0.0.1:$http_port" --grpc "127.0.0.1:$grpc_port" \
  >"$work/server.log" 2>&1 &
server_pid=$!
# The API site is the default server for 127.0.0.1; gRPC is selected by SNI "localhost".
sed -e "s#/etc/geoledger/tls/#$work/#g" \
    -e "s#server geoledger:7881#server 127.0.0.1:$http_port#" \
    -e "s#server geoledger:7882#server 127.0.0.1:$grpc_port#" \
    -e "s#server console:3000#server 127.0.0.1:9#" \
    -e "s#listen 443 ssl;#listen $tls_port ssl;#" \
    -e "/listen \[::\]/d" \
    -e "s#listen 80;#listen $plain_port;#" \
    -e "s#server_name api.geoledger.example;#server_name 127.0.0.1;#" \
    -e "s#server_name grpc.geoledger.example;#server_name localhost;#" \
    -e "s#access_log /dev/stdout gl;#access_log $work/access.log gl;#" \
    deploy/gateway/nginx.conf > "$work/nginx.conf"
sed -i "1i pid $work/nginx.pid;\nerror_log $work/error.log;" "$work/nginx.conf"
sed -i "s#^http {#http {\n    client_body_temp_path $work/body;\n    proxy_temp_path $work/proxy;\n    fastcgi_temp_path $work/fcgi;\n    uwsgi_temp_path $work/uwsgi;\n    scgi_temp_path $work/scgi;#" "$work/nginx.conf"
"$nginx_bin" -t -c "$work/nginx.conf" -p "$work" 2>&1 | grep -q "test is successful"
"$nginx_bin" -c "$work/nginx.conf" -p "$work"
for _ in $(seq 100); do [[ -f "$work/data/admin-credentials.json" ]] && break; sleep 0.1; done
for _ in $(seq 100); do curl -fsS --cacert "$work/ca.crt" "https://127.0.0.1:$tls_port/ready" >/dev/null 2>&1 && break; sleep 0.1; done
headers=$(curl -fsS -D - -o /dev/null --cacert "$work/ca.crt" "https://127.0.0.1:$tls_port/ready")
grep -qi '^strict-transport-security:' <<<"$headers"
grep -qi '^x-request-id:' <<<"$headers"
test "$(curl -s -o /dev/null -w '%{http_code}' "http://127.0.0.1:$plain_port/ready")" = 301
info=$("$cli_bin" --endpoint "https://localhost:$tls_port" --ca-file "$work/ca.crt" \
  --token-file "$work/data/admin-credentials.json" info)
grep -q '"backend": "sqlite"' <<<"$info"
echo "gateway TLS verified: HTTP API, HSTS, request IDs, HTTP->HTTPS redirect and gRPC"
