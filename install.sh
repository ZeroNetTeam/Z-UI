#!/usr/bin/env bash
# Z-UI installer: a whole server, from nothing to a panel you can sign in to.
#
#   bash <(curl -Ls https://github.com/ZeroNetTeam/Z-UI/releases/latest/download/install.sh)
#
# It checks the machine, fetches a release and checks its hash (and, with
# OpenSSL 3, its signature), makes the first admin and a random port and
# secret path, can prepare the server (BBR, the clock, the firewall), can get
# an HTTPS certificate for a domain (or this server's address), and starts the
# panel as a systemd service. Run it again to update; `z-ui` is the menu that
# manages it afterwards. See --help.
#
# For tests, ZUI_PREFIX puts every path under another root, ZUI_NO_SYSTEMD=1
# leaves systemd alone, and --binary installs a program you already have.

set -Eeuo pipefail

REPO="ZeroNetTeam/Z-UI"
RELEASE_BASE="${ZUI_RELEASE_BASE:-https://github.com/${REPO}/releases}"
# The public half of the key releases are signed with. The panel has the same
# key built in (a test keeps the two equal).
SIGNING_KEY="GwpOuUsuF4bQ2KIA4TEqJDS716OT2Q4PiSY9GOYHogs="

ROOT="${ZUI_PREFIX:-}"
BIN_DIR="${ROOT}/usr/local/z-ui"
DATA_DIR="${ROOT}/var/lib/z-ui"
ETC_DIR="${ROOT}/etc/z-ui"
UNIT="${ROOT}/etc/systemd/system/z-ui.service"
MENU="${ROOT}/usr/local/bin/z-ui"
STATE="${ETC_DIR}/install.state"

# ----------------------------------------------------------------- messages
declare -A EN FA

EN[root]="Run this as root (sudo bash install.sh)."
EN[arch]="This kind of machine (%s) has no build. Z-UI is built for x86_64 and aarch64."
EN[os_unknown]="Could not tell the distribution; going on, since it is Linux with systemd."
EN[os_ok]="System: %s (%s)"
EN[os_untested]="%s is not one Z-UI is tried on; going on."
EN[no_systemd]="systemd is not running here. Z-UI installs as a systemd service."
EN[disk]="Not enough disk space: %s MB free in %s, 300 MB needed."
EN[missing]="Installing what is missing: %s"
EN[cannot_install]="Cannot install %s automatically; install it and run this again."
EN[language]="Language"
EN[existing]="Z-UI is already installed here: updating the program and keeping your data and settings."
EN[ask_port]="Panel port"
EN[ask_path]="Secret path of the panel"
EN[ask_user]="Admin username"
EN[ask_pass]="Admin password (empty: a random one)"
EN[ask_domain]="Domain for HTTPS (empty: none, plain HTTP)"
EN[ask_email]="Email for the certificate authority (empty: none)"
EN[port_busy]="Port %s is in use."
EN[downloading]="Downloading Z-UI %s (%s)"
EN[download_failed]="Could not download %s. If GitHub is closed from this server, use --proxy or --mirror."
EN[hash_bad]="The downloaded file does not match its hash. Nothing was installed."
EN[hash_ok]="Hash checked."
EN[sig_ok]="Signature of the release checked."
EN[sig_bad]="The signature of this release is wrong. Nothing was installed."
EN[sig_skipped]="Signature not checked here (OpenSSL 3 is needed); the hash was. The panel checks signatures itself when it updates."
EN[installed]="Installed %s"
EN[admin_made]="Made the admin %s"
EN[bbr_on]="BBR congestion control is on."
EN[bbr_no]="This kernel has no BBR; skipped."
EN[ntp_on]="The clock is kept in sync."
EN[ntp_no]="Could not turn on time sync; check the clock (Reality and 2FA need it right)."
EN[fw_open]="Opened %s in %s."
EN[fw_none]="No active firewall found (ufw or firewalld); if your provider has one, open the ports there: %s"
EN[dns_none]="%s has no address in DNS yet; point its A record at %s first."
EN[dns_other]="%s points to %s, not to this server (%s). Certificates cannot be got until it does."
EN[dns_cdn]="%s is behind a CDN proxy (Cloudflare's orange cloud). HTTP and TLS challenges cannot work; use --cert-method dns-01 with --cloudflare-token."
EN[cert_try]="Getting a certificate for %s ..."
EN[cert_ok]="Certificate #%s is in place; the panel serves HTTPS."
EN[cert_failed]="No certificate yet: %s"
EN[cert_later]="The panel runs on plain HTTP for now. Try again later with: z-ui cert"
EN[started]="The panel is running."
EN[not_started]="The panel did not answer in time. See: journalctl -u z-ui -e"
EN[done]="Z-UI is ready"
EN[url]="Address"
EN[user]="Username"
EN[pass]="Password"
EN[pass_kept]="(your existing admin; reset it with: z-ui)"
EN[menu_hint]="Manage it with the command: z-ui"
EN[data]="Data"
EN[uninstall_confirm]="Remove Z-UI from this server?"
EN[uninstall_data]="Also delete the data (users, certificates, backups)?"
EN[uninstalled]="Z-UI was removed."
EN[kept_data]="The data is kept in %s"
EN[unknown_option]="Unknown option: %s (see --help)"
EN[help]="Usage: install.sh [options]

  -y, --yes               no questions: use the options and defaults
      --lang fa|en        language of messages
      --version vX.Y.Z    install this release (default: the latest)
      --port N            panel port (default: random)
      --path /secret      secret path (default: random)
      --username NAME     first admin (default: random)
      --password PASS     its password (default: random, shown at the end)
      --domain NAME       get an HTTPS certificate for this domain
      --email ADDRESS     email for the certificate authority
      --cert-method M     auto | http-01 | tls-alpn-01 | dns-01
      --cloudflare-token T  for dns-01 and wildcard names
      --staging           use the authority's test server
      --ip-cert           get a short-lived certificate for this server's address
      --no-cert           do not try to get a certificate
      --no-firewall       leave the firewall alone
      --no-bbr            leave the kernel's congestion control alone
      --no-ntp            leave time sync alone
      --proxy URL         proxy for downloads (http://, socks5://)
      --mirror URL        use this address in place of GitHub's releases
      --binary FILE       install this program, do not download
      --uninstall         remove Z-UI (--purge: and its data)
  -h, --help"

FA[root]="این را با دسترسی root اجرا کنید (sudo bash install.sh)."
FA[arch]="برای این نوع ماشین (%s) نسخه‌ای ساخته نشده. Z-UI برای x86_64 و aarch64 ساخته می‌شود."
FA[os_unknown]="توزیع را نشناختم؛ ادامه می‌دهم چون لینوکس با systemd است."
FA[os_ok]="سیستم: %s (%s)"
FA[os_untested]="Z-UI روی %s آزموده نشده؛ ادامه می‌دهم."
FA[no_systemd]="اینجا systemd اجرا نمی‌شود. Z-UI به‌صورت سرویس systemd نصب می‌شود."
FA[disk]="فضای دیسک کافی نیست: %s مگابایت آزاد در %s، ۳۰۰ مگابایت لازم است."
FA[missing]="نصب آنچه کم است: %s"
FA[cannot_install]="%s را نمی‌شود خودکار نصب کرد؛ خودتان نصب کنید و دوباره اجرا کنید."
FA[language]="زبان"
FA[existing]="Z-UI از قبل اینجا نصب است: برنامه به‌روز می‌شود و داده و تنظیمات شما می‌ماند."
FA[ask_port]="پورت پنل"
FA[ask_path]="مسیر مخفی پنل"
FA[ask_user]="نام کاربری ادمین"
FA[ask_pass]="رمز ادمین (خالی: تصادفی)"
FA[ask_domain]="دامنه برای HTTPS (خالی: بدون HTTPS، HTTP ساده)"
FA[ask_email]="ایمیل برای مرجع صدور گواهی (خالی: بدون ایمیل)"
FA[port_busy]="پورت %s پر است."
FA[downloading]="دانلود Z-UI %s (%s)"
FA[download_failed]="%s دانلود نشد. اگر GitHub از این سرور بسته است از --proxy یا --mirror استفاده کنید."
FA[hash_bad]="فایل دانلودشده با هشش نمی‌خواند. چیزی نصب نشد."
FA[hash_ok]="هش بررسی شد."
FA[sig_ok]="امضای انتشار بررسی شد."
FA[sig_bad]="امضای این انتشار اشتباه است. چیزی نصب نشد."
FA[sig_skipped]="امضا اینجا بررسی نشد (OpenSSL 3 لازم است)؛ هش بررسی شد. پنل هنگام به‌روزرسانی خودش امضا را می‌سنجد."
FA[installed]="نصب شد: %s"
FA[admin_made]="ادمین %s ساخته شد"
FA[bbr_on]="کنترل ازدحام BBR روشن شد."
FA[bbr_no]="این هسته BBR ندارد؛ رد شد."
FA[ntp_on]="ساعت همگام نگه داشته می‌شود."
FA[ntp_no]="همگام‌سازی ساعت روشن نشد؛ ساعت را بررسی کنید (Reality و 2FA ساعت درست می‌خواهند)."
FA[fw_open]="%s در %s باز شد."
FA[fw_none]="فایروال فعالی (ufw یا firewalld) پیدا نشد؛ اگر ارائه‌دهنده‌ی شما فایروال دارد پورت‌ها را آنجا باز کنید: %s"
FA[dns_none]="%s هنوز در DNS آدرسی ندارد؛ اول رکورد A آن را به %s بزنید."
FA[dns_other]="%s به %s اشاره می‌کند، نه به این سرور (%s). تا این درست نشود گواهی گرفته نمی‌شود."
FA[dns_cdn]="%s پشت پروکسی CDN است (ابر نارنجی Cloudflare). چالش HTTP و TLS کار نمی‌کند؛ از --cert-method dns-01 با --cloudflare-token استفاده کنید."
FA[cert_try]="در حال گرفتن گواهی برای %s ..."
FA[cert_ok]="گواهی #%s سر جایش است؛ پنل با HTTPS سرو می‌شود."
FA[cert_failed]="هنوز گواهی نیست: %s"
FA[cert_later]="پنل فعلاً با HTTP ساده کار می‌کند. بعداً دوباره امتحان کنید: z-ui cert"
FA[started]="پنل در حال اجراست."
FA[not_started]="پنل به‌موقع جواب نداد. ببینید: journalctl -u z-ui -e"
FA[done]="Z-UI آماده است"
FA[url]="آدرس"
FA[user]="نام کاربری"
FA[pass]="رمز عبور"
FA[pass_kept]="(ادمین قبلی شما؛ با دستور z-ui بازنشانی کنید)"
FA[menu_hint]="با دستور z-ui مدیریتش کنید"
FA[data]="داده"
FA[uninstall_confirm]="Z-UI از این سرور حذف شود؟"
FA[uninstall_data]="داده (کاربران، گواهی‌ها، بکاپ‌ها) هم پاک شود؟"
FA[uninstalled]="Z-UI حذف شد."
FA[kept_data]="داده در %s نگه داشته شد"
FA[unknown_option]="گزینه‌ی ناشناخته: %s (--help را ببینید)"
FA[help]="روش استفاده: install.sh [گزینه‌ها]

  -y, --yes               بدون پرسش: با گزینه‌ها و پیش‌فرض‌ها
      --lang fa|en        زبان پیام‌ها
      --version vX.Y.Z    این انتشار را نصب کن (پیش‌فرض: آخرین)
      --port N            پورت پنل (پیش‌فرض: تصادفی)
      --path /secret      مسیر مخفی (پیش‌فرض: تصادفی)
      --username NAME     ادمین اول (پیش‌فرض: تصادفی)
      --password PASS     رمزش (پیش‌فرض: تصادفی، در پایان نشان داده می‌شود)
      --domain NAME       برای این دامنه گواهی HTTPS بگیر
      --email ADDRESS     ایمیل برای مرجع صدور
      --cert-method M     auto | http-01 | tls-alpn-01 | dns-01
      --cloudflare-token T  برای dns-01 و نام‌های wildcard
      --staging           از سرور آزمایشی مرجع استفاده کن
      --ip-cert           برای آدرس این سرور گواهی کوتاه‌مدت بگیر
      --no-cert           گواهی نگیر
      --no-firewall       به فایروال دست نزن
      --no-bbr            به کنترل ازدحام هسته دست نزن
      --no-ntp            به همگام‌سازی ساعت دست نزن
      --proxy URL         پراکسی برای دانلود (http://، socks5://)
      --mirror URL        این آدرس را به‌جای انتشارهای GitHub به کار ببر
      --binary FILE       این برنامه را نصب کن، دانلود نکن
      --uninstall         حذف Z-UI (--purge: و داده‌اش)
  -h, --help"

LANG_CODE="${ZUI_LANG:-}"

# Print message $1 in the language chosen, with the other arguments as its
# printf arguments.
msg() {
  local key="$1"
  shift
  local text
  if [ "$LANG_CODE" = "fa" ]; then text="${FA[$key]:-${EN[$key]:-$key}}"; else text="${EN[$key]:-$key}"; fi
  # shellcheck disable=SC2059
  printf "$text" "$@"
}

c_reset=$'\033[0m'; c_green=$'\033[32m'; c_yellow=$'\033[33m'; c_red=$'\033[31m'; c_bold=$'\033[1m'
if [ ! -t 1 ]; then c_reset=""; c_green=""; c_yellow=""; c_red=""; c_bold=""; fi

info() { printf '%s\n' "$(msg "$@")"; }
ok() { printf '%s✓%s %s\n' "$c_green" "$c_reset" "$(msg "$@")"; }
warn() { printf '%s!%s %s\n' "$c_yellow" "$c_reset" "$(msg "$@")" >&2; }
die() { printf '%s✗%s %s\n' "$c_red" "$c_reset" "$(msg "$@")" >&2; exit 1; }

# ------------------------------------------------------------------- options
YES=0 VERSION="" PORT="" URL_PATH="" ADMIN_USER="" ADMIN_PASS="" DOMAIN="" EMAIL=""
CERT_METHOD="auto" CF_TOKEN="" STAGING=0 IP_CERT=0 NO_CERT=0 NO_FW=0 NO_BBR=0 NO_NTP=0
PROXY="" MIRROR="" BINARY="${ZUI_BINARY:-}" UNINSTALL=0 PURGE=0
PASS_GENERATED=0 UPGRADE=0

parse_args() {
  while [ $# -gt 0 ]; do
    case "$1" in
      -y|--yes) YES=1 ;;
      --lang) LANG_CODE="${2:?}"; shift ;;
      --version) VERSION="${2:?}"; shift ;;
      --port) PORT="${2:?}"; shift ;;
      --path) URL_PATH="${2:?}"; shift ;;
      --username) ADMIN_USER="${2:?}"; shift ;;
      --password) ADMIN_PASS="${2:?}"; shift ;;
      --domain) DOMAIN="${2:?}"; shift ;;
      --email) EMAIL="${2:?}"; shift ;;
      --cert-method) CERT_METHOD="${2:?}"; shift ;;
      --cloudflare-token) CF_TOKEN="${2:?}"; shift ;;
      --staging) STAGING=1 ;;
      --ip-cert) IP_CERT=1 ;;
      --no-cert) NO_CERT=1 ;;
      --no-firewall) NO_FW=1 ;;
      --no-bbr) NO_BBR=1 ;;
      --no-ntp) NO_NTP=1 ;;
      --proxy) PROXY="${2:?}"; shift ;;
      --mirror) MIRROR="${2:?}"; shift ;;
      --binary) BINARY="${2:?}"; shift ;;
      --uninstall) UNINSTALL=1 ;;
      --purge) PURGE=1 ;;
      -h|--help) set_language; msg help; echo; exit 0 ;;
      *) set_language; die unknown_option "$1" ;;
    esac
    shift
  done
}

# ------------------------------------------------------------------- helpers
interactive() { [ "$YES" -eq 0 ] && { [ -t 0 ] || [ -r /dev/tty ]; }; }

# ask VAR "message key" [default]: read a line (from the terminal even when
# the script itself is piped in), keeping the default on an empty answer.
ask() {
  local var="$1" key="$2" default="${3:-}" answer=""
  if ! interactive; then printf -v "$var" '%s' "${!var:-$default}"; return; fi
  printf '%s%s%s' "$c_bold" "$(msg "$key")" "$c_reset" >&2
  [ -n "$default" ] && printf ' [%s]' "$default" >&2
  printf ': ' >&2
  if [ -t 0 ]; then read -r answer || true; else read -r answer </dev/tty || true; fi
  printf -v "$var" '%s' "${answer:-${!var:-$default}}"
}

set_language() {
  if [ -z "$LANG_CODE" ]; then
    case "${LC_ALL:-${LANG:-}}" in fa*|FA*) LANG_CODE=fa ;; *) LANG_CODE=en ;; esac
    if interactive; then
      local choice=""
      printf 'Language / زبان: 1) English  2) فارسی [%s]: ' "$([ "$LANG_CODE" = fa ] && echo 2 || echo 1)" >&2
      if [ -t 0 ]; then read -r choice || true; else read -r choice </dev/tty || true; fi
      case "$choice" in 1) LANG_CODE=en ;; 2) LANG_CODE=fa ;; esac
    fi
  fi
  case "$LANG_CODE" in fa|en) ;; *) LANG_CODE=en ;; esac
}

need_root() {
  [ "${ZUI_SKIP_ROOT_CHECK:-0}" = 1 ] && return 0
  [ "$(id -u)" -eq 0 ] || die root
}

# amd64 or arm64, from the kernel's name for the machine.
detect_arch() {
  case "${ZUI_FAKE_ARCH:-$(uname -m)}" in
    x86_64|amd64) echo amd64 ;;
    aarch64|arm64) echo arm64 ;;
    *) return 1 ;;
  esac
}

# Sets OS_ID, OS_NAME and PM (the package manager) from os-release.
detect_os() {
  local file="${ZUI_OS_RELEASE:-/etc/os-release}" id="" like="" name=""
  OS_ID="" OS_NAME="" PM="none"
  if [ -r "$file" ]; then
    # shellcheck disable=SC1090
    id="$(. "$file"; echo "${ID:-}")"; like="$(. "$file"; echo "${ID_LIKE:-}")"; name="$(. "$file"; echo "${PRETTY_NAME:-${NAME:-}}")"
  fi
  OS_ID="$id" OS_NAME="$name"
  case " $id $like " in
    *" ubuntu "*|*" debian "*) PM=apt ;;
    *" fedora "*) PM=dnf ;;
    *" rhel "*|*" centos "*|*" rocky "*|*" almalinux "*) command -v dnf >/dev/null 2>&1 && PM=dnf || PM=yum ;;
    *" arch "*|*" archlinux "*) PM=pacman ;;
  esac
  [ -n "$id" ]
}

check_system() {
  local arch
  arch="$(detect_arch)" || die arch "$(uname -m)"
  ARCH="$arch"
  if detect_os; then
    ok os_ok "${OS_NAME:-$OS_ID}" "$ARCH"
    case "$OS_ID" in
      ubuntu|debian|centos|rocky|almalinux|fedora|arch|rhel|ol|linuxmint|raspbian|manjaro|endeavouros) ;;
      *) [ "$PM" = none ] && warn os_untested "${OS_NAME:-$OS_ID}" ;;
    esac
  else
    warn os_unknown
  fi
  if [ "${ZUI_NO_SYSTEMD:-0}" != 1 ] && [ ! -d /run/systemd/system ]; then die no_systemd; fi
  local probe="$ROOT/" free
  [ -d "$probe" ] || probe="/"
  free="$(df -Pk "$probe" 2>/dev/null | awk 'NR==2 {print int($4/1024)}')"
  if [ -n "${free:-}" ] && [ "$free" -lt 300 ]; then die disk "$free" "$probe"; fi
}

# Install what is missing with the system's package manager.
ensure_tools() {
  local missing=() tool
  command -v curl >/dev/null 2>&1 || command -v wget >/dev/null 2>&1 || missing+=(curl)
  for tool in tar sha256sum; do command -v "$tool" >/dev/null 2>&1 || missing+=("$tool"); done
  [ "${#missing[@]}" -eq 0 ] && return 0
  info missing "${missing[*]}"
  local pkgs=()
  for tool in "${missing[@]}"; do
    case "$tool" in sha256sum) pkgs+=(coreutils) ;; *) pkgs+=("$tool") ;; esac
  done
  case "$PM" in
    apt) apt-get update -qq && DEBIAN_FRONTEND=noninteractive apt-get install -y -qq "${pkgs[@]}" ca-certificates ;;
    dnf) dnf install -y -q "${pkgs[@]}" ca-certificates ;;
    yum) yum install -y -q "${pkgs[@]}" ca-certificates ;;
    pacman) pacman -Sy --noconfirm --needed "${pkgs[@]}" ca-certificates ;;
    *) die cannot_install "${missing[*]}" ;;
  esac
}

# Whether TCP port $1 has nobody on it.
port_free() {
  local port="$1"
  if command -v ss >/dev/null 2>&1; then
    [ -z "$(ss -ltnH "sport = :$port" 2>/dev/null)" ]
  else
    ! (exec 3<>"/dev/tcp/127.0.0.1/$port") 2>/dev/null
  fi
}

random_hex() { od -An -N"$(( ($1 + 1) / 2 ))" -tx1 /dev/urandom | tr -d ' \n' | cut -c1-"$1"; }

random_port() {
  local port tries=0
  while [ "$tries" -lt 50 ]; do
    port=$(( 10000 + $(od -An -N2 -tu2 /dev/urandom | tr -d ' ') % 50000 ))
    port_free "$port" && { echo "$port"; return 0; }
    tries=$((tries + 1))
  done
  echo 24680
}

# /secret from whatever was typed: letters, digits, - _ . and /.
normalise_path() {
  local p="$1"
  p="${p#/}"; p="${p%/}"
  [ -n "$p" ] || { echo "/"; return; }
  case "$p" in *[!A-Za-z0-9._/-]*) return 1 ;; esac
  echo "/$p"
}

# The address other machines reach this one by.
public_ip() {
  local url ip
  for url in https://api.ipify.org https://ifconfig.me/ip https://icanhazip.com; do
    ip="$(curl -fsS4 --max-time 5 ${PROXY:+--proxy "$PROXY"} "$url" 2>/dev/null | tr -d ' \r\n' || true)"
    case "$ip" in ''|*[!0-9.]*) ;; *) echo "$ip"; return 0 ;; esac
  done
  ip route get 1.1.1.1 2>/dev/null | awk '{for (i = 1; i < NF; i++) if ($i == "src") {print $(i + 1); exit}}'
}

# The IPv4 and IPv6 addresses a name has, one per line.
resolve_ips() {
  if command -v getent >/dev/null 2>&1; then
    getent ahosts "$1" 2>/dev/null | awk '{print $1}' | sort -u
  elif command -v dig >/dev/null 2>&1; then
    { dig +short A "$1"; dig +short AAAA "$1"; } 2>/dev/null | sort -u
  elif command -v host >/dev/null 2>&1; then
    host "$1" 2>/dev/null | awk '/has (IPv6 )?address/ {print $NF}' | sort -u
  fi
}

ip_to_int() { local IFS=.; read -r a b c d <<<"$1"; echo $(( (a << 24) + (b << 16) + (c << 8) + d )); }

# Whether address $1 is one of Cloudflare's published ranges (what a proxied,
# "orange cloud" name resolves to).
is_cloudflare() {
  local ip="$1" range net bits mask
  case "$ip" in
    *:*)
      case "$ip" in 2400:cb00:*|2606:4700:*|2803:f800:*|2405:b500:*|2405:8100:*|2a06:98c0:*|2c0f:f248:*) return 0 ;; esac
      return 1 ;;
  esac
  for range in 173.245.48.0/20 103.21.244.0/22 103.22.200.0/22 103.31.4.0/22 141.101.64.0/18 108.162.192.0/18 \
               190.93.240.0/20 188.114.96.0/20 197.234.240.0/22 198.41.128.0/17 162.158.0.0/15 104.16.0.0/13 \
               104.24.0.0/14 172.64.0.0/13 131.0.72.0/22; do
    net="${range%/*}"; bits="${range#*/}"
    mask=$(( (0xFFFFFFFF << (32 - bits)) & 0xFFFFFFFF ))
    if [ $(( $(ip_to_int "$ip") & mask )) -eq $(( $(ip_to_int "$net") & mask )) ]; then return 0; fi
  done
  return 1
}

# Say what is wrong with $1 as a domain for a certificate, and return 1; or
# return 0 when it points at this server (SERVER_IP).
check_domain() {
  local name="$1" ips ip any_other=0 cdn=0
  ips="$(resolve_ips "$name")"
  if [ -z "$ips" ]; then warn dns_none "$name" "${SERVER_IP:-?}"; return 1; fi
  while IFS= read -r ip; do
    [ -z "$ip" ] && continue
    if is_cloudflare "$ip"; then cdn=1; fi
    case "$ip" in *:*) ;; *) [ "$ip" = "${SERVER_IP:-}" ] || any_other=1 ;; esac
  done <<<"$ips"
  if [ "$cdn" -eq 1 ]; then warn dns_cdn "$name"; return 1; fi
  if [ "$any_other" -eq 1 ]; then warn dns_other "$name" "$(echo "$ips" | tr '\n' ' ' | sed 's/ $//')" "${SERVER_IP:-?}"; return 1; fi
  return 0
}

download() {
  local url="$1" dest="$2"
  if command -v curl >/dev/null 2>&1; then
    curl -fL --retry 3 --connect-timeout 15 ${PROXY:+--proxy "$PROXY"} -o "$dest" "$url"
  else
    wget -q ${PROXY:+-e "https_proxy=$PROXY"} -O "$dest" "$url"
  fi
}

release_url() {
  local asset="$1" base="${MIRROR:-$RELEASE_BASE}"
  if [ -n "$VERSION" ]; then echo "${base%/}/download/${VERSION}/${asset}"; else echo "${base%/}/latest/download/${asset}"; fi
}

# Whether OpenSSL can check an Ed25519 signature of a file (version 3).
openssl_can_verify() {
  command -v openssl >/dev/null 2>&1 && openssl version 2>/dev/null | grep -Eq 'OpenSSL (3|[4-9])\.'
}

# Check the signature of SHA256SUMS in $1 (a directory), when we can.
verify_signature() {
  local dir="$1"
  if ! openssl_can_verify || [ ! -f "$dir/SHA256SUMS.sig" ]; then warn sig_skipped; return 0; fi
  local key_der="$dir/key.der" key_pem="$dir/key.pem" sig="$dir/sig.bin"
  { printf '\x30\x2a\x30\x05\x06\x03\x2b\x65\x70\x03\x21\x00'; printf '%s' "$SIGNING_KEY" | base64 -d; } >"$key_der"
  openssl pkey -pubin -inform DER -in "$key_der" -out "$key_pem" 2>/dev/null
  # The signature file holds the 64 bytes, raw or in base64.
  if [ "$(wc -c <"$dir/SHA256SUMS.sig")" -eq 64 ]; then cp "$dir/SHA256SUMS.sig" "$sig"; else base64 -d "$dir/SHA256SUMS.sig" >"$sig" 2>/dev/null || return 1; fi
  if openssl pkeyutl -verify -pubin -inkey "$key_pem" -rawin -in "$dir/SHA256SUMS" -sigfile "$sig" >/dev/null 2>&1; then
    ok sig_ok
  else
    return 1
  fi
}

# Fetch the release into $1 and leave the program as $1/z-ui and the menu as $1/z-ui.sh.
fetch_release() {
  local work="$1" asset="z-ui-linux-${ARCH}.tar.gz"
  if [ -n "$BINARY" ]; then
    [ -f "$BINARY" ] || die download_failed "$BINARY"
    cp "$BINARY" "$work/z-ui"
    # From a checkout, the menu is the one beside this script.
    local menu="${ZUI_MENU_SCRIPT:-$(dirname "${BASH_SOURCE[0]}")/deploy/z-ui.sh}"
    if [ -f "$menu" ]; then cp "$menu" "$work/z-ui.sh"; fi
    return 0
  fi
  info downloading "${VERSION:-latest}" "$asset"
  local f
  for f in "$asset" SHA256SUMS; do
    download "$(release_url "$f")" "$work/$f" || die download_failed "$f"
  done
  download "$(release_url SHA256SUMS.sig)" "$work/SHA256SUMS.sig" 2>/dev/null || rm -f "$work/SHA256SUMS.sig"
  verify_signature "$work" || die sig_bad
  local expected got
  expected="$(awk -v f="$asset" '{n=$2; sub(/^\*/, "", n)} n == f {print tolower($1)}' "$work/SHA256SUMS")"
  got="$(sha256sum "$work/$asset" | awk '{print $1}')"
  [ -n "$expected" ] && [ "$expected" = "$got" ] || die hash_bad
  ok hash_ok
  tar -xzf "$work/$asset" -C "$work"
  local found
  found="$(find "$work" -type f -name z-ui -not -path "$work/z-ui" | head -n1)"
  [ -n "$found" ] || die download_failed "$asset"
  mv "$found" "$work/z-ui"
  found="$(find "$work" -type f -name z-ui.sh | head -n1 || true)"
  [ -n "$found" ] && [ "$found" != "$work/z-ui.sh" ] && mv "$found" "$work/z-ui.sh"
  return 0
}

# --------------------------------------------------------------- the install
zui() { ZUI_DATA_DIR="$DATA_DIR" "$BIN_DIR/z-ui" "$@"; }

remember() { mkdir -p "$ETC_DIR"; echo "$*" >>"$STATE"; }

install_files() {
  local work="$1"
  mkdir -p "$BIN_DIR" "$DATA_DIR" "$ETC_DIR" "$(dirname "$MENU")" "$(dirname "$UNIT")"
  chmod 700 "$DATA_DIR"
  # A replaced program is renamed over the old one, so a running one is not cut off.
  install -m 0755 "$work/z-ui" "$BIN_DIR/z-ui.new"
  [ -f "$BIN_DIR/z-ui" ] && cp "$BIN_DIR/z-ui" "$BIN_DIR/z-ui.prev"
  mv -f "$BIN_DIR/z-ui.new" "$BIN_DIR/z-ui"
  if [ -f "$work/z-ui.sh" ]; then install -m 0755 "$work/z-ui.sh" "$MENU"; fi
  # Where SELinux is enforcing, systemd runs only what is labelled as a program.
  if command -v selinuxenabled >/dev/null 2>&1 && selinuxenabled 2>/dev/null && command -v chcon >/dev/null 2>&1; then
    chcon -R -t bin_t "$BIN_DIR" 2>/dev/null || true
  fi
  [ -f "$ETC_DIR/z-ui.env" ] || printf 'ZUI_DATA_DIR=%s\n' "/var/lib/z-ui" >"$ETC_DIR/z-ui.env"
  printf '%s\n' "$LANG_CODE" >"$ETC_DIR/lang"
  ok installed "$BIN_DIR/z-ui"
}

write_unit() {
  cat >"$UNIT" <<UNIT
[Unit]
Description=Z-UI panel
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
EnvironmentFile=-/etc/z-ui/z-ui.env
WorkingDirectory=/var/lib/z-ui
ExecStart=/usr/local/z-ui/z-ui
Restart=always
RestartSec=3
LimitNOFILE=1048576

[Install]
WantedBy=multi-user.target
UNIT
}

systemd_do() { [ "${ZUI_NO_SYSTEMD:-0}" = 1 ] || systemctl "$@"; }

prepare_bbr() {
  [ "$NO_BBR" -eq 1 ] && return 0
  [ "${ZUI_NO_SYSTEMD:-0}" = 1 ] && return 0
  if ! sysctl -n net.ipv4.tcp_available_congestion_control 2>/dev/null | grep -qw bbr; then
    modprobe tcp_bbr 2>/dev/null || true
  fi
  if sysctl -n net.ipv4.tcp_available_congestion_control 2>/dev/null | grep -qw bbr; then
    local file="${ROOT}/etc/sysctl.d/99-z-ui-bbr.conf"
    mkdir -p "$(dirname "$file")"
    printf 'net.core.default_qdisc=fq\nnet.ipv4.tcp_congestion_control=bbr\n' >"$file"
    sysctl -q -p "$file" >/dev/null 2>&1 || true
    remember "file $file"
    ok bbr_on
  else
    warn bbr_no
  fi
}

prepare_time() {
  [ "$NO_NTP" -eq 1 ] && return 0
  [ "${ZUI_NO_SYSTEMD:-0}" = 1 ] && return 0
  if command -v timedatectl >/dev/null 2>&1 && timedatectl set-ntp true >/dev/null 2>&1; then
    ok ntp_on
  else
    warn ntp_no
  fi
}

# Open TCP ports $@ in the firewall that is on, and remember it for uninstalling.
open_ports() {
  [ "$NO_FW" -eq 1 ] && return 0
  [ "${ZUI_NO_SYSTEMD:-0}" = 1 ] && return 0
  local port done_any=0
  if command -v ufw >/dev/null 2>&1 && ufw status 2>/dev/null | grep -q '^Status: active'; then
    for port in "$@"; do ufw allow "${port}/tcp" >/dev/null && remember "ufw ${port}/tcp" && ok fw_open "${port}/tcp" ufw; done
    done_any=1
  elif command -v firewall-cmd >/dev/null 2>&1 && firewall-cmd --state >/dev/null 2>&1; then
    for port in "$@"; do firewall-cmd --permanent --add-port="${port}/tcp" >/dev/null && remember "firewalld ${port}/tcp" && ok fw_open "${port}/tcp" firewalld; done
    firewall-cmd --reload >/dev/null
    done_any=1
  fi
  [ "$done_any" -eq 1 ] || warn fw_none "$*"
}

wait_for_panel() {
  local i
  [ "${ZUI_NO_SYSTEMD:-0}" = 1 ] && return 0
  for i in $(seq 1 30); do
    if zui health >/dev/null 2>&1; then return 0; fi
    sleep 1
  done
  return 1
}

# Get a certificate for $1 and make the panel serve with it. Returns 1 (with a
# word said) if it could not.
get_certificate() {
  local names="$1" out id
  info cert_try "$names"
  local args=(cert issue "$names" --method "$CERT_METHOD" --porcelain)
  [ -n "$EMAIL" ] && args+=(--email "$EMAIL")
  [ -n "$CF_TOKEN" ] && args+=(--cloudflare-token "$CF_TOKEN")
  [ "$STAGING" -eq 1 ] && args+=(--ca letsencrypt_staging)
  if ! out="$(zui "${args[@]}" 2>&1)"; then
    warn cert_failed "$(printf '%s' "$out" | tail -n 3 | tr '\n' ' ')"
    return 1
  fi
  id="$(printf '%s' "$out" | tail -n1 | tr -dc '0-9')"
  [ -n "$id" ] || { warn cert_failed "$out"; return 1; }
  zui setting set --certificate "$id" >/dev/null
  ok cert_ok "$id"
}

do_install() {
  local work
  work="$(mktemp -d)"
  # Expanded now: the variable is gone by the time the script exits.
  # shellcheck disable=SC2064
  trap "rm -rf '$work'" EXIT
  [ -f "$DATA_DIR/z-ui.db" ] && UPGRADE=1 || UPGRADE=0
  [ "$UPGRADE" -eq 1 ] && info existing

  if [ "$UPGRADE" -eq 0 ]; then
    ask PORT ask_port "${PORT:-$(random_port)}"
    while ! port_free "$PORT"; do
      warn port_busy "$PORT"
      [ "$YES" -eq 1 ] && die port_busy "$PORT"
      PORT=""; ask PORT ask_port "$(random_port)"
    done
    ask URL_PATH ask_path "${URL_PATH:-/$(random_hex 12)}"
    URL_PATH="$(normalise_path "$URL_PATH")" || die unknown_option "$URL_PATH"
    ask ADMIN_USER ask_user "${ADMIN_USER:-$(random_hex 8)}"
    ask ADMIN_PASS ask_pass ""
    if [ -z "$ADMIN_PASS" ]; then ADMIN_PASS="$(random_hex 18)"; PASS_GENERATED=1; fi
    if [ "$NO_CERT" -eq 0 ] && [ "$IP_CERT" -eq 0 ]; then
      ask DOMAIN ask_domain "${DOMAIN:-}"
      [ -n "$DOMAIN" ] && ask EMAIL ask_email "${EMAIL:-}"
    fi
  fi

  ensure_tools
  fetch_release "$work"
  install_files "$work"
  write_unit

  if [ "$UPGRADE" -eq 0 ]; then
    zui admin create "$ADMIN_USER" --password "$ADMIN_PASS" >/dev/null
    ok admin_made "$ADMIN_USER"
    zui setting set --port "$PORT" --path "$URL_PATH" >/dev/null
    prepare_bbr
    prepare_time
    SERVER_IP="$(public_ip || true)"
    local ports=("$PORT") want_cert=0
    [ "$NO_CERT" -eq 0 ] && { [ -n "$DOMAIN" ] || [ "$IP_CERT" -eq 1 ]; } && want_cert=1
    [ "$want_cert" -eq 1 ] && ports+=(80 443)
    open_ports "${ports[@]}"
    if [ "$want_cert" -eq 1 ]; then
      local target
      if [ -n "$DOMAIN" ]; then target="$DOMAIN"; else target="$SERVER_IP"; fi
      if { [ -z "$DOMAIN" ] || check_domain "$DOMAIN"; } && get_certificate "$target"; then :; else info cert_later; fi
    fi
  fi

  systemd_do daemon-reload
  systemd_do enable z-ui >/dev/null 2>&1 || true
  systemd_do restart z-ui
  if wait_for_panel; then ok started; else warn not_started; fi
  summary
}

summary() {
  local scheme="http" host port path shown
  local setting
  setting="$(zui setting show 2>/dev/null || true)"
  port="$(printf '%s\n' "$setting" | awk '/^listen/ {n=split($2, a, ":"); print a[n]}')"
  path="$(printf '%s\n' "$setting" | awk '/^path/ {print $2}')"
  printf '%s\n' "$setting" | grep -q '^certificate  #' && scheme="https"
  host="${DOMAIN:-${SERVER_IP:-$(public_ip || echo "<server>")}}"
  [ -n "$DOMAIN" ] || [ "$IP_CERT" -eq 0 ] || host="${SERVER_IP:-$host}"
  [ "$path" = "/" ] && shown="/" || shown="${path}/"
  echo
  printf '%s%s%s\n' "$c_green" "$(msg done)" "$c_reset"
  printf '  %s: %s%s://%s:%s%s%s\n' "$(msg url)" "$c_bold" "$scheme" "$host" "${port:-?}" "$shown" "$c_reset"
  if [ "$UPGRADE" -eq 0 ]; then
    printf '  %s: %s\n' "$(msg user)" "$ADMIN_USER"
    printf '  %s: %s\n' "$(msg pass)" "$ADMIN_PASS"
  else
    printf '  %s %s\n' "$(msg user)" "$(msg pass_kept)"
  fi
  printf '  %s: %s\n' "$(msg data)" "/var/lib/z-ui"
  printf '  %s\n' "$(msg menu_hint)"
}

# --------------------------------------------------------------- uninstalling
# The menu knows how to undo what was done; this is for an install that lost it.
do_uninstall() {
  if [ -x "$MENU" ]; then
    local args=(uninstall)
    [ "$YES" -eq 1 ] && args+=(-y)
    [ "$PURGE" -eq 1 ] && args+=(--purge)
    exec "$MENU" "${args[@]}"
  fi
  systemd_do stop z-ui 2>/dev/null || true
  systemd_do disable z-ui 2>/dev/null || true
  rm -f "$UNIT"
  systemd_do daemon-reload 2>/dev/null || true
  rm -rf "$BIN_DIR" "$ETC_DIR"
  [ "$PURGE" -eq 1 ] && rm -rf "$DATA_DIR"
  ok uninstalled
}

main() {
  parse_args "$@"
  set_language
  need_root
  if [ "$UNINSTALL" -eq 1 ]; then do_uninstall; exit 0; fi
  check_system
  do_install
}

# Sourced by the tests, which call the functions one by one.
if [ "${ZUI_INSTALL_SOURCE_ONLY:-0}" != 1 ]; then main "$@"; fi
