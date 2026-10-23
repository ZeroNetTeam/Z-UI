#!/usr/bin/env bash
# `z-ui`: the menu that manages an installed Z-UI panel.
#
#   z-ui                      the menu
#   z-ui start|stop|restart|status|log
#   z-ui update | rollback    to the newest version, or back to the last one
#   z-ui setting              where the panel is (address, path, certificate)
#   z-ui port N | path /p     change them (the panel restarts)
#   z-ui cert DOMAIN          get an HTTPS certificate for a domain and use it
#   z-ui password [USER]      a new password for an admin (also turns 2FA off with --2fa)
#   z-ui backup | restore FILE
#   z-ui uninstall | version | lang fa|en | help
#
# Add -y anywhere to answer every question "yes".

set -Eeuo pipefail

ROOT="${ZUI_PREFIX:-}"
BIN_DIR="${ROOT}/usr/local/z-ui"
DATA_DIR="${ROOT}/var/lib/z-ui"
ETC_DIR="${ROOT}/etc/z-ui"
UNIT="${ROOT}/etc/systemd/system/z-ui.service"
BIN="${BIN_DIR}/z-ui"

declare -A EN FA
EN[title]="Z-UI panel management"
EN[m1]="Start"; EN[m2]="Stop"; EN[m3]="Restart"; EN[m4]="Status"; EN[m5]="Logs"
EN[m6]="Update"; EN[m7]="Roll back the last update"; EN[m8]="Show address and settings"
EN[m9]="Change the port"; EN[m10]="Change the secret path"; EN[m11]="Domain and HTTPS certificate"
EN[m12]="Reset an admin's password"; EN[m13]="Back up now"; EN[m14]="Restore a backup"
EN[m15]="Language"; EN[m16]="Uninstall"; EN[m0]="Exit"
EN[choose]="Choose"
EN[invalid]="That is not a choice."
EN[restart_now]="Restart the panel now so this takes effect?"
EN[restarted]="The panel was restarted."
EN[need_value]="This needs a value: z-ui %s <value>"
EN[bad_path]="The path is letters, digits, - _ . and /."
EN[bad_port]="The port is a number from 1 to 65535."
EN[ask_port]="New port"
EN[ask_path]="New secret path"
EN[ask_domain]="Domain (its A record must point at this server)"
EN[ask_email]="Email for the certificate authority (empty: none)"
EN[ask_user]="Admin username"
EN[ask_file]="Backup file to restore"
EN[ask_2fa]="Also turn two-factor sign-in off?"
EN[confirm_update]="Update to the newest version?"
EN[no_unit]="There is no Z-UI service here; is it installed?"
EN[uninstall_confirm]="Remove Z-UI from this server?"
EN[uninstall_data]="Also delete the data (users, certificates, backups)?"
EN[uninstalled]="Z-UI was removed."
EN[kept_data]="The data is kept in %s"
EN[sure_restore]="Everything the panel keeps is replaced by this backup (the present state is saved first). Stop the panel, restore, start it?"
EN[restored]="Restored; the panel was started again."
EN[lang_set]="Language set."
EN[help]="Usage: z-ui [command]

  (none)             the menu
  start | stop | restart | status | log
  update | rollback  to the newest version | back to the last one
  setting            address, path and certificate in use
  port N | path /p   change them
  cert DOMAIN [--email E] [--method M] [--staging] [--cloudflare-token T]
                     get an HTTPS certificate and use it
  password [USER] [--2fa]   a new password (a random one is shown)
  backup | restore FILE
  uninstall [--purge]   remove it (--purge: the data too)
  version | lang fa|en
  -y                 answer yes to every question"

FA[title]="مدیریت پنل Z-UI"
FA[m1]="شروع"; FA[m2]="توقف"; FA[m3]="راه‌اندازی دوباره"; FA[m4]="وضعیت"; FA[m5]="لاگ‌ها"
FA[m6]="به‌روزرسانی"; FA[m7]="برگشت به‌روزرسانی قبلی"; FA[m8]="نمایش آدرس و تنظیمات"
FA[m9]="تغییر پورت"; FA[m10]="تغییر مسیر مخفی"; FA[m11]="دامنه و گواهی HTTPS"
FA[m12]="بازنشانی رمز یک ادمین"; FA[m13]="بکاپ همین حالا"; FA[m14]="بازگرداندن بکاپ"
FA[m15]="زبان"; FA[m16]="حذف"; FA[m0]="خروج"
FA[choose]="انتخاب"
FA[invalid]="این گزینه‌ای نیست."
FA[restart_now]="پنل الان دوباره راه‌اندازی شود تا اعمال شود؟"
FA[restarted]="پنل دوباره راه‌اندازی شد."
FA[need_value]="این یک مقدار می‌خواهد: z-ui %s <مقدار>"
FA[bad_path]="مسیر فقط حرف، رقم و - _ . و / است."
FA[bad_port]="پورت عددی از ۱ تا ۶۵۵۳۵ است."
FA[ask_port]="پورت تازه"
FA[ask_path]="مسیر مخفی تازه"
FA[ask_domain]="دامنه (رکورد A آن باید به این سرور اشاره کند)"
FA[ask_email]="ایمیل برای مرجع صدور گواهی (خالی: بدون ایمیل)"
FA[ask_user]="نام کاربری ادمین"
FA[ask_file]="فایل بکاپی که باید برگردد"
FA[ask_2fa]="ورود دومرحله‌ای هم خاموش شود؟"
FA[confirm_update]="به آخرین نسخه به‌روز شود؟"
FA[no_unit]="سرویس Z-UI اینجا نیست؛ نصب شده است؟"
FA[uninstall_confirm]="Z-UI از این سرور حذف شود؟"
FA[uninstall_data]="داده (کاربران، گواهی‌ها، بکاپ‌ها) هم پاک شود؟"
FA[uninstalled]="Z-UI حذف شد."
FA[kept_data]="داده در %s نگه داشته شد"
FA[sure_restore]="هرچه پنل نگه می‌دارد با این بکاپ عوض می‌شود (وضعیت فعلی اول ذخیره می‌شود). پنل متوقف، بکاپ برگردانده و پنل شروع شود؟"
FA[restored]="برگردانده شد و پنل دوباره شروع شد."
FA[lang_set]="زبان تنظیم شد."
FA[help]="روش استفاده: z-ui [فرمان]

  (هیچ)              منو
  start | stop | restart | status | log
  update | rollback  به آخرین نسخه | برگشت به نسخه‌ی قبلی
  setting            آدرس، مسیر و گواهی در حال استفاده
  port N | path /p   تغییرشان
  cert DOMAIN [--email E] [--method M] [--staging] [--cloudflare-token T]
                     گواهی HTTPS بگیر و به کار ببر
  password [USER] [--2fa]   رمز تازه (رمز تصادفی نشان داده می‌شود)
  backup | restore FILE
  uninstall [--purge]   حذف (--purge: داده هم)
  version | lang fa|en
  -y                 به همه‌ی پرسش‌ها بله"

LANG_CODE="${ZUI_LANG:-}"
[ -n "$LANG_CODE" ] || { [ -r "$ETC_DIR/lang" ] && LANG_CODE="$(cat "$ETC_DIR/lang")" || LANG_CODE=en; }
case "$LANG_CODE" in fa|en) ;; *) LANG_CODE=en ;; esac

msg() {
  local key="$1"; shift
  local text
  if [ "$LANG_CODE" = fa ]; then text="${FA[$key]:-${EN[$key]:-$key}}"; else text="${EN[$key]:-$key}"; fi
  # shellcheck disable=SC2059
  printf "$text" "$@"
}

YES=0
ARGS=()
for arg in "$@"; do if [ "$arg" = "-y" ] || [ "$arg" = "--yes" ]; then YES=1; else ARGS+=("$arg"); fi; done
set -- "${ARGS[@]+"${ARGS[@]}"}"

zui() { ZUI_DATA_DIR="$DATA_DIR" "$BIN" "$@"; }
systemd_do() { if [ "${ZUI_NO_SYSTEMD:-0}" = 1 ]; then return 0; fi; systemctl "$@"; }

confirm() {
  [ "$YES" -eq 1 ] && return 0
  local answer=""
  printf '%s [y/N]: ' "$(msg "$1")" >&2
  read -r answer || true
  case "$answer" in y|Y|yes|Yes) return 0 ;; *) return 1 ;; esac
}

prompt() { # prompt VAR key [default]
  local var="$1" key="$2" default="${3:-}" answer=""
  printf '%s%s: ' "$(msg "$key")" "${default:+ [$default]}" >&2
  read -r answer || true
  printf -v "$var" '%s' "${answer:-$default}"
}

restart_panel() {
  systemd_do restart z-ui
  msg restarted; echo
}

# After a change that needs a restart.
offer_restart() { if confirm restart_now; then restart_panel; fi; }

need_unit() { [ "${ZUI_NO_SYSTEMD:-0}" = 1 ] || [ -f "$UNIT" ] || { msg no_unit; echo >&2; exit 1; }; }

valid_port() { case "$1" in ''|*[!0-9]*) return 1 ;; esac; [ "$1" -ge 1 ] && [ "$1" -le 65535 ]; }
valid_path() { case "${1#/}" in *[!A-Za-z0-9._/-]*) return 1 ;; esac; }

cmd_setting() { zui setting show; }

cmd_port() {
  local port="${1:-}"
  [ -n "$port" ] || { prompt port ask_port; }
  valid_port "$port" || { msg bad_port; echo >&2; exit 1; }
  zui setting set --port "$port"
  offer_restart
}

cmd_path() {
  local path="${1:-}"
  [ -n "$path" ] || { prompt path ask_path; }
  valid_path "$path" || { msg bad_path; echo >&2; exit 1; }
  zui setting set --path "$path"
  offer_restart
}

cmd_cert() {
  local domain="" email="" method=auto staging=0 token=""
  while [ $# -gt 0 ]; do
    case "$1" in
      --email) email="${2:?}"; shift ;;
      --method) method="${2:?}"; shift ;;
      --staging) staging=1 ;;
      --cloudflare-token) token="${2:?}"; shift ;;
      *) domain="$1" ;;
    esac
    shift
  done
  [ -n "$domain" ] || prompt domain ask_domain
  if [ -z "$email" ] && [ "$YES" -eq 0 ]; then prompt email ask_email; fi
  local args=(cert issue "$domain" --method "$method" --porcelain)
  [ -n "$email" ] && args+=(--email "$email")
  [ -n "$token" ] && args+=(--cloudflare-token "$token")
  [ "$staging" -eq 1 ] && args+=(--ca letsencrypt_staging)
  local id
  id="$(zui "${args[@]}" | tail -n1 | tr -dc '0-9')"
  zui setting set --certificate "$id"
  zui cert list
  offer_restart
}

cmd_password() {
  local user="" off=0
  while [ $# -gt 0 ]; do case "$1" in --2fa) off=1 ;; *) user="$1" ;; esac; shift; done
  [ -n "$user" ] || prompt user ask_user
  if [ "$off" -eq 0 ] && [ "$YES" -eq 0 ] && confirm ask_2fa; then off=1; fi
  if [ "$off" -eq 1 ]; then zui admin reset-password "$user" --disable-2fa; else zui admin reset-password "$user"; fi
}

cmd_update() {
  zui update check
  if confirm confirm_update; then
    zui update apply --yes
    restart_panel
  fi
}

cmd_restore() {
  local file="${1:-}"
  [ -n "$file" ] || prompt file ask_file
  confirm sure_restore || exit 0
  systemd_do stop z-ui
  zui restore "$file"
  systemd_do start z-ui
  msg restored; echo
}

cmd_lang() {
  local lang="${1:-}"
  case "$lang" in fa|en) ;; *) [ "$LANG_CODE" = fa ] && lang=en || lang=fa ;; esac
  mkdir -p "$ETC_DIR"
  printf '%s\n' "$lang" >"$ETC_DIR/lang"
  LANG_CODE="$lang"
  msg lang_set; echo
}

cmd_uninstall() {
  local purge=0 arg
  for arg in "$@"; do [ "$arg" = "--purge" ] && purge=1; done
  confirm uninstall_confirm || return 0
  systemd_do stop z-ui 2>/dev/null || true
  systemd_do disable z-ui 2>/dev/null || true
  rm -f "$UNIT"
  systemd_do daemon-reload 2>/dev/null || true
  # What the installer changed, undone: the firewall ports it opened and the
  # files it made elsewhere.
  if [ -f "$ETC_DIR/install.state" ]; then
    local kind what
    while read -r kind what; do
      case "$kind" in
        ufw) ufw delete allow "$what" >/dev/null 2>&1 || true ;;
        firewalld) firewall-cmd --permanent --remove-port="$what" >/dev/null 2>&1 || true ;;
        file) rm -f "$what" ;;
      esac
    done <"$ETC_DIR/install.state"
    if command -v firewall-cmd >/dev/null 2>&1; then firewall-cmd --reload >/dev/null 2>&1 || true; fi
  fi
  rm -rf "$BIN_DIR" "$ETC_DIR"
  if [ "$purge" -eq 0 ] && [ "$YES" -eq 0 ] && confirm uninstall_data; then purge=1; fi
  if [ "$purge" -eq 1 ]; then rm -rf "$DATA_DIR"; elif [ -d "$DATA_DIR" ]; then msg kept_data "$DATA_DIR"; echo; fi
  # Last, because this is the program that is running.
  rm -f "${ROOT}/usr/local/bin/z-ui"
  msg uninstalled; echo
}

menu() {
  local choice
  while true; do
    echo
    printf '%s%s%s\n' $'\033[1m' "$(msg title) — $(zui --version 2>/dev/null | head -n1)" $'\033[0m'
    local i
    for i in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16; do printf '  %2d) %s\n' "$i" "$(msg "m$i")"; done
    printf '   0) %s\n' "$(msg m0)"
    printf '%s: ' "$(msg choose)" >&2
    read -r choice || exit 0
    case "$choice" in
      1) systemd_do start z-ui ;;
      2) systemd_do stop z-ui ;;
      3) restart_panel ;;
      4) systemd_do status z-ui --no-pager || true; zui health || true ;;
      5) journalctl -u z-ui -n 100 --no-pager || true ;;
      6) cmd_update ;;
      7) zui update rollback; offer_restart ;;
      8) cmd_setting ;;
      9) cmd_port ;;
      10) cmd_path ;;
      11) cmd_cert ;;
      12) cmd_password ;;
      13) zui backup ;;
      14) cmd_restore ;;
      15) cmd_lang ;;
      16) cmd_uninstall; exit 0 ;;
      0|q|"") exit 0 ;;
      *) msg invalid; echo ;;
    esac
  done
}

main() {
  local command="${1:-menu}"
  [ $# -gt 0 ] && shift
  case "$command" in
    menu) need_unit; menu ;;
    start|stop|enable|disable) need_unit; systemd_do "$command" z-ui ;;
    restart) need_unit; restart_panel ;;
    status) systemd_do status z-ui --no-pager || true; zui health || true ;;
    log|logs) journalctl -u z-ui -n "${1:-100}" --no-pager ;;
    update) cmd_update ;;
    rollback) zui update rollback; offer_restart ;;
    setting|settings) cmd_setting ;;
    port) cmd_port "$@" ;;
    path) cmd_path "$@" ;;
    cert|domain) cmd_cert "$@" ;;
    password) cmd_password "$@" ;;
    admin) zui admin "$@" ;;
    backup) zui backup "$@" ;;
    restore) cmd_restore "$@" ;;
    uninstall) cmd_uninstall "$@" ;;
    lang) cmd_lang "$@" ;;
    version|--version|-v) zui --version ;;
    help|--help|-h) msg help; echo ;;
    *) msg help >&2; echo >&2; exit 2 ;;
  esac
}

if [ "${ZUI_MENU_SOURCE_ONLY:-0}" != 1 ]; then main "$@"; fi
