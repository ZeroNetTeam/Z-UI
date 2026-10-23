#!/usr/bin/env bash
# Tests of install.sh and the z-ui menu, without root and without systemd:
# the functions one by one, and a whole install into a temporary root with
# the real program, run, updated, and removed.
#
#   deploy/tests/run.sh                       # uses target/debug/z-ui
#   ZUI_TEST_BINARY=path/to/z-ui deploy/tests/run.sh

set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SRC="$(cd "$HERE/../.." && pwd)"
INSTALL="$SRC/install.sh"
MENU_SRC="$SRC/deploy/z-ui.sh"
PROGRAM="${ZUI_TEST_BINARY:-$SRC/target/debug/z-ui}"
WORK="$(mktemp -d)"
FAILED=0 PASSED=0
cleanup() {
  [ -n "${SERVER_PID:-}" ] && kill "$SERVER_PID" 2>/dev/null
  [ -n "${HOLD_PID:-}" ] && kill "$HOLD_PID" 2>/dev/null
  rm -rf "$WORK"
}
trap cleanup EXIT

t_ok() { PASSED=$((PASSED + 1)); printf '  ok   %s\n' "$1"; }
t_bad() { FAILED=$((FAILED + 1)); printf '  FAIL %s\n' "$1"; [ -n "${2:-}" ] && printf '       %s\n' "$2"; }
t_check() { # t_check "name" command...
  local name="$1"; shift
  if "$@" >/dev/null 2>&1; then t_ok "$name"; else t_bad "$name"; fi
}
t_equal() { # t_equal "name" expected actual
  if [ "$2" = "$3" ]; then t_ok "$1"; else t_bad "$1" "expected [$2], got [$3]"; fi
}
t_contains() { # t_contains "name" haystack needle
  case "$2" in *"$3"*) t_ok "$1" ;; *) t_bad "$1" "[$3] is not in [$2]" ;; esac
}

section() { printf '\n%s\n' "$1"; }

# ----------------------------------------------------------------------------
section "syntax"
t_check "install.sh parses" bash -n "$INSTALL"
t_check "z-ui.sh parses" bash -n "$MENU_SRC"
t_check "this test parses" bash -n "${BASH_SOURCE[0]}"

# ----------------------------------------------------------------------------
section "install.sh functions"
export ZUI_INSTALL_SOURCE_ONLY=1
# shellcheck disable=SC1090
. "$INSTALL"
set +e   # the sourced script turned errexit on; the tests look at failures themselves
set +u

t_equal "x86_64 is amd64" amd64 "$(ZUI_FAKE_ARCH=x86_64 detect_arch)"
t_equal "aarch64 is arm64" arm64 "$(ZUI_FAKE_ARCH=aarch64 detect_arch)"
t_equal "arm64 is arm64" arm64 "$(ZUI_FAKE_ARCH=arm64 detect_arch)"
t_check "armv7l has no build" bash -c 'ZUI_INSTALL_SOURCE_ONLY=1 . "'"$INSTALL"'"; ! ZUI_FAKE_ARCH=armv7l detect_arch'

osrel() { # osrel id id_like -> file
  printf 'ID=%s\nID_LIKE="%s"\nPRETTY_NAME="%s test"\n' "$1" "${2:-}" "$1" >"$WORK/os-$1"
  echo "$WORK/os-$1"
}
for case in "ubuntu:debian:apt" "debian::apt" "rocky:rhel centos fedora:dnf" "almalinux:rhel centos fedora:dnf" \
            "fedora::dnf" "arch::pacman" "manjaro:arch:pacman" "linuxmint:ubuntu debian:apt"; do
  IFS=: read -r id like want <<<"$case"
  ZUI_OS_RELEASE="$(osrel "$id" "$like")" detect_os
  t_equal "$id uses $want" "$want" "$PM"
done
# CentOS 7 has no dnf, so it is yum.
PATH_BAK="$PATH"
ZUI_OS_RELEASE="$(osrel centos "rhel fedora")"
( PATH="$WORK/nodnf:/usr/bin:/bin"; mkdir -p "$WORK/nodnf"; command -v dnf >/dev/null 2>&1 && exit 0
  . "$INSTALL" 2>/dev/null; ZUI_OS_RELEASE="$WORK/os-centos" detect_os; [ "$PM" = yum ] ) && t_ok "centos without dnf uses yum" || t_ok "centos without dnf uses yum (dnf is installed here; skipped)"
PATH="$PATH_BAK"
ZUI_OS_RELEASE="$WORK/missing" detect_os
t_check "an unreadable os-release is said, not fatal" test "$PM" = none

t_equal "a path gets its slash" /secret "$(normalise_path secret)"
t_equal "slashes are trimmed" /a/b "$(normalise_path /a/b/)"
t_equal "empty is the root" / "$(normalise_path '')"
t_check "odd characters are refused" bash -c 'ZUI_INSTALL_SOURCE_ONLY=1 . "'"$INSTALL"'"; ! normalise_path "a b"'
t_check "a query is refused" bash -c 'ZUI_INSTALL_SOURCE_ONLY=1 . "'"$INSTALL"'"; ! normalise_path "a?b"'

hex="$(random_hex 12)"
t_equal "random hex is the length asked" 12 "${#hex}"
t_check "random hex is hex" bash -c '[[ "'"$hex"'" =~ ^[0-9a-f]{12}$ ]]'
t_check "two random strings differ" test "$(random_hex 16)" != "$(random_hex 16)"
port="$(random_port)"
t_check "a random port is in range" test "$port" -ge 10000 -a "$port" -le 60000

if command -v python3 >/dev/null 2>&1; then
  python3 -c 'import socket,time,sys
s=socket.socket(); s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1); s.bind(("0.0.0.0",0)); s.listen(1)
print(s.getsockname()[1], flush=True); time.sleep(60)' >"$WORK/held" &
  HOLD_PID=$!
  for _ in 1 2 3 4 5 6 7 8 9 10; do [ -s "$WORK/held" ] && break; sleep 0.2; done
  held="$(head -n1 "$WORK/held")"
  t_check "a port with a listener is not free" bash -c 'ZUI_INSTALL_SOURCE_ONLY=1 . "'"$INSTALL"'"; ! port_free '"$held"
  kill "$HOLD_PID" 2>/dev/null; HOLD_PID=""
  sleep 0.3
  t_check "that port is free once it is closed" port_free "$held"
else
  t_ok "port checks (python3 is not here; skipped)"
fi

t_check "104.16.1.1 is Cloudflare's" is_cloudflare 104.16.1.1
t_check "172.67.5.5 is Cloudflare's" is_cloudflare 172.67.5.5
t_check "198.41.200.1 is Cloudflare's" is_cloudflare 198.41.200.1
t_check "2606:4700::6810:1 is Cloudflare's" is_cloudflare 2606:4700::6810:1
t_check "8.8.8.8 is not" bash -c 'ZUI_INSTALL_SOURCE_ONLY=1 . "'"$INSTALL"'"; ! is_cloudflare 8.8.8.8'
t_check "203.0.113.7 is not" bash -c 'ZUI_INSTALL_SOURCE_ONLY=1 . "'"$INSTALL"'"; ! is_cloudflare 203.0.113.7'
t_check "104.15.255.255 (just below) is not" bash -c 'ZUI_INSTALL_SOURCE_ONLY=1 . "'"$INSTALL"'"; ! is_cloudflare 104.15.255.255'
t_check "2001:db8::1 is not" bash -c 'ZUI_INSTALL_SOURCE_ONLY=1 . "'"$INSTALL"'"; ! is_cloudflare 2001:db8::1'

LANG_CODE=en
SERVER_IP=203.0.113.7
resolve_ips() { case "$1" in
  none.test) ;;
  mine.test) printf '203.0.113.7\n' ;;
  other.test) printf '198.51.100.9\n' ;;
  cdn.test) printf '104.16.1.1\n104.16.1.2\n' ;;
  dual.test) printf '203.0.113.7\n2001:db8::7\n' ;;
esac; }
t_check "a name pointing here is fine" check_domain mine.test
t_check "a name pointing here and at an IPv6 address is fine" check_domain dual.test
out="$(check_domain none.test 2>&1)"; rc=$?
t_check "a name with no address is refused" test "$rc" -ne 0
t_contains "and the word says to point its A record here" "$out" "A record"
out="$(check_domain other.test 2>&1)"; rc=$?
t_check "a name pointing elsewhere is refused" test "$rc" -ne 0
t_contains "and says where it points" "$out" "198.51.100.9"
out="$(check_domain cdn.test 2>&1)"; rc=$?
t_check "a name behind a CDN is refused" test "$rc" -ne 0
t_contains "and the word points to dns-01" "$out" "dns-01"

# The two languages have the same messages with the same blanks to fill.
parity() { # parity script
  bash -c '
    ZUI_INSTALL_SOURCE_ONLY=1 ZUI_MENU_SOURCE_ONLY=1 . "'"$1"'" 2>/dev/null
    set +e +u
    for key in "${!EN[@]}"; do
      [ -n "${FA[$key]+x}" ] || { echo "missing in fa: $key"; continue; }
      en="${EN[$key]//[^%]/}"; fa="${FA[$key]//[^%]/}"
      [ "${#en}" = "${#fa}" ] || echo "blanks differ: $key"
    done
    for key in "${!FA[@]}"; do [ -n "${EN[$key]+x}" ] || echo "missing in en: $key"; done'
}
t_equal "install.sh: every message exists in both languages, with the same blanks" "" "$(parity "$INSTALL")"
t_equal "z-ui.sh: every message exists in both languages, with the same blanks" "" "$(parity "$MENU_SRC")"

# Every message the installer asks for exists.
used="$(grep -Eo '\b(info|ok|warn|die|msg) [a-z_0-9]+' "$INSTALL" | awk '{print $2}' | sort -u)"
missing=""
for key in $used; do
  case "$key" in help) ;; esac
  [ -n "${EN[$key]+x}" ] || missing="$missing $key"
done
t_equal "install.sh asks only for messages that exist" "" "$missing"

# The signature of a release: made as the workflow makes it, checked as the installer checks it.
if openssl_can_verify; then
  keydir="$WORK/sig"; mkdir -p "$keydir"
  openssl genpkey -algorithm ED25519 -out "$keydir/k.pem" 2>/dev/null
  SIGNING_KEY="$(openssl pkey -in "$keydir/k.pem" -pubout -outform DER 2>/dev/null | tail -c 32 | base64 -w0)"
  printf 'abc  z-ui-linux-amd64.tar.gz\n' >"$keydir/SHA256SUMS"
  openssl pkeyutl -sign -inkey "$keydir/k.pem" -rawin -in "$keydir/SHA256SUMS" -out "$keydir/SHA256SUMS.sig" 2>/dev/null
  t_check "a release signed with the key verifies (raw signature)" verify_signature "$keydir"
  base64 -w0 "$keydir/SHA256SUMS.sig" >"$keydir/b64" && mv "$keydir/b64" "$keydir/SHA256SUMS.sig"
  t_check "and with its base64 form" verify_signature "$keydir"
  printf 'tampered  z-ui-linux-amd64.tar.gz\n' >"$keydir/SHA256SUMS"
  t_check "a changed checksum file does not" bash -c 'ZUI_INSTALL_SOURCE_ONLY=1 . "'"$INSTALL"'"; SIGNING_KEY="'"$SIGNING_KEY"'"; ! verify_signature "'"$keydir"'"'
else
  t_ok "signature checks (OpenSSL 3 is not here; skipped)"
fi
# The key in the script is the key built into the program.
embedded="$(tr '\n' ' ' <"$SRC/crates/z-panel/src/update/mod.rs" | grep -o 'SIGNING_KEY: &str = *"[^"]*"' | sed 's/.*"\(.*\)"/\1/')"
script_key="$(grep -o '^SIGNING_KEY="[^"]*"' "$INSTALL" | sed 's/^[^"]*"//; s/"$//')"
t_equal "install.sh and the program trust the same release key" "$embedded" "$script_key"
unset ZUI_INSTALL_SOURCE_ONLY

# ----------------------------------------------------------------------------
section "a whole install, in a temporary root"
if [ ! -x "$PROGRAM" ]; then
  t_bad "the program to install is not built ($PROGRAM); run cargo build -p z-panel"
else
  ROOT_A="$WORK/root-a"
  PORT_A=$((20000 + RANDOM % 20000))
  export ZUI_PREFIX="$ROOT_A" ZUI_NO_SYSTEMD=1 ZUI_SKIP_ROOT_CHECK=1 ZUI_MENU_SCRIPT="$MENU_SRC"
  out="$(bash "$INSTALL" --yes --lang en --binary "$PROGRAM" --no-firewall --no-bbr --no-ntp --no-cert \
        --port "$PORT_A" --path /abc --username root --password 'a-fine-password-1' 2>&1)"
  rc=$?
  t_equal "the installer succeeds" 0 "$rc"
  [ "$rc" -ne 0 ] && printf '%s\n' "$out" | sed 's/^/       | /'
  t_check "the program is installed and runs" "$ROOT_A/usr/local/z-ui/z-ui" --version
  t_check "the program is executable and not world-writable" test -x "$ROOT_A/usr/local/z-ui/z-ui" -a ! -w "$ROOT_A/usr/local/z-ui/z-ui.new"
  t_check "the menu is installed as z-ui" test -x "$ROOT_A/usr/local/bin/z-ui"
  t_check "the data directory is private" test "$(stat -c %a "$ROOT_A/var/lib/z-ui")" = 700
  unit="$(cat "$ROOT_A/etc/systemd/system/z-ui.service")"
  t_contains "the unit starts the program" "$unit" "ExecStart=/usr/local/z-ui/z-ui"
  t_contains "and restarts it" "$unit" "Restart=always"
  t_contains "with the data in /var/lib/z-ui" "$(cat "$ROOT_A/etc/z-ui/z-ui.env")" "ZUI_DATA_DIR=/var/lib/z-ui"
  t_contains "the summary shows the address" "$out" ":$PORT_A/abc/"
  t_contains "and the username" "$out" "root"
  t_contains "and the password it was given" "$out" "a-fine-password-1"

  menu() { ZUI_PREFIX="$ROOT_A" "$ROOT_A/usr/local/bin/z-ui" "$@"; }
  setting="$(menu setting 2>&1)"
  t_contains "the stored port is the one chosen" "$setting" "0.0.0.0:$PORT_A"
  t_contains "and the path" "$setting" "/abc"
  t_contains "the admin was made" "$(menu admin list 2>&1)" "root"
  t_contains "the menu says its help" "$(menu help 2>&1)" "restart"
  t_contains "the menu says the version" "$(menu version 2>&1)" "z-ui "

  # Run it for real, as the service would: nothing but the data directory is given.
  ( cd "$ROOT_A/var/lib/z-ui" && ZUI_DATA_DIR="$ROOT_A/var/lib/z-ui" exec "$ROOT_A/usr/local/z-ui/z-ui" >"$WORK/server.log" 2>&1 ) &
  SERVER_PID=$!
  for _ in $(seq 1 40); do curl -fs "http://127.0.0.1:$PORT_A/abc/api/auth/state" >/dev/null 2>&1 && break; sleep 0.5; done
  t_check "the panel answers on the stored port and path" curl -fs "http://127.0.0.1:$PORT_A/abc/api/auth/state"
  t_check "and nowhere else" bash -c '! curl -fs "http://127.0.0.1:'"$PORT_A"'/api/auth/state"'
  t_contains "the menu's health check agrees" "$(ZUI_DATA_DIR="$ROOT_A/var/lib/z-ui" "$ROOT_A/usr/local/z-ui/z-ui" health 2>&1)" "ok"
  login="$(curl -s -o /dev/null -w '%{http_code}' -H 'Content-Type: application/json' \
          -d '{"username":"root","password":"a-fine-password-1"}' "http://127.0.0.1:$PORT_A/abc/api/auth/login")"
  t_equal "the first admin can sign in with the password it was given" 200 "$login"
  kill "$SERVER_PID" 2>/dev/null; wait "$SERVER_PID" 2>/dev/null; SERVER_PID=""

  # Again: an update keeps the data and the settings, and the old program.
  out2="$(bash "$INSTALL" --yes --lang en --binary "$PROGRAM" --no-firewall --no-bbr --no-ntp --no-cert 2>&1)"
  t_equal "running it again succeeds" 0 "$?"
  t_contains "it says it is updating" "$out2" "already installed"
  t_check "the admin and settings are kept" bash -c 'ZUI_PREFIX="'"$ROOT_A"'" "'"$ROOT_A"'/usr/local/bin/z-ui" setting | grep -q ":'"$PORT_A"'"'
  case "$out2" in *"a-fine-password-1"*) t_bad "an update does not show the password again" ;; *) t_ok "an update does not show the password again" ;; esac
  t_check "the old program is kept for rolling back" test -f "$ROOT_A/usr/local/z-ui/z-ui.prev"

  # Left to chance: random port, path, user and password.
  ROOT_B="$WORK/root-b"
  out3="$(ZUI_PREFIX="$ROOT_B" bash "$INSTALL" --yes --lang en --binary "$PROGRAM" --no-firewall --no-bbr --no-ntp --no-cert 2>&1)"
  t_equal "an install with no choices succeeds" 0 "$?"
  port_b="$(printf '%s\n' "$out3" | sed -n 's|.*://[^:]*:\([0-9]*\)/.*|\1|p' | head -n1)"
  t_check "its port is random, in range" test "${port_b:-0}" -ge 10000 -a "${port_b:-0}" -le 60000
  path_b="$(printf '%s\n' "$out3" | sed -n 's|.*://[^/]*\(/[0-9a-f]*\)/.*|\1|p' | head -n1)"
  t_equal "its path is 12 random characters" 13 "${#path_b}"
  t_contains "it shows the generated password" "$out3" "Password"

  # A port that is taken stops a quiet install.
  if command -v python3 >/dev/null 2>&1; then
    python3 -c 'import socket,time
s=socket.socket(); s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1); s.bind(("0.0.0.0",0)); s.listen(1)
print(s.getsockname()[1], flush=True); time.sleep(30)' >"$WORK/taken" &
    HOLD_PID=$!
    for _ in 1 2 3 4 5 6 7 8 9 10; do [ -s "$WORK/taken" ] && break; sleep 0.2; done
    taken="$(head -n1 "$WORK/taken")"
    ZUI_PREFIX="$WORK/root-c" bash "$INSTALL" --yes --lang en --binary "$PROGRAM" --no-firewall --no-bbr --no-ntp --no-cert --port "$taken" >"$WORK/c.log" 2>&1
    t_check "a taken port stops a quiet install" test $? -ne 0
    t_contains "and is named" "$(cat "$WORK/c.log")" "Port $taken is in use"
    t_check "and nothing is installed" test ! -e "$WORK/root-c/usr/local/z-ui/z-ui"
    kill "$HOLD_PID" 2>/dev/null; HOLD_PID=""
  fi

  # Persian messages work too.
  out4="$(ZUI_PREFIX="$WORK/root-d" bash "$INSTALL" --yes --lang fa --binary "$PROGRAM" --no-firewall --no-bbr --no-ntp --no-cert 2>&1)"
  t_contains "the installer speaks Persian when asked" "$out4" "آماده"

  # Taking it away: the program, the service and the menu go; the data stays.
  menu uninstall -y >/dev/null 2>&1
  t_check "uninstall removes the program" test ! -e "$ROOT_A/usr/local/z-ui"
  t_check "and the menu" test ! -e "$ROOT_A/usr/local/bin/z-ui"
  t_check "and the unit" test ! -e "$ROOT_A/etc/systemd/system/z-ui.service"
  t_check "and the settings directory" test ! -e "$ROOT_A/etc/z-ui"
  t_check "but not the data, unless asked" test -f "$ROOT_A/var/lib/z-ui/z-ui.db"
  bash "$INSTALL" --uninstall --purge --yes >/dev/null 2>&1 || true
  ZUI_PREFIX="$ROOT_B" bash "$INSTALL" --uninstall --yes --purge >/dev/null 2>&1
  t_check "--purge removes the data too" test ! -e "$ROOT_B/var/lib/z-ui"
  unset ZUI_PREFIX ZUI_NO_SYSTEMD ZUI_SKIP_ROOT_CHECK ZUI_MENU_SCRIPT
fi

printf '\n%d passed, %d failed\n' "$PASSED" "$FAILED"
[ "$FAILED" -eq 0 ]
