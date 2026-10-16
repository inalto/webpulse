# shellcheck shell=bash
# WebPulse installer library. Sourced by install.sh (never executed directly).
#
# Entry points: wp_install, wp_uninstall. Everything is driven by the WP_*
# variables set by install.sh and by the WEBPULSE_* environment overrides that
# make the whole flow testable in a temp directory without privileges:
#
#   WEBPULSE_PREFIX         prepended to every system path (default: empty = real system)
#   WEBPULSE_FAKE_CPANEL=1  treat WEBPULSE_CPANEL_ROOT as /usr/local/cpanel, skip the root check
#   WEBPULSE_CPANEL_ROOT    cPanel root (default: $PREFIX/usr/local/cpanel)
#   WEBPULSE_ETC            config dir   (default: $PREFIX/etc/webpulse)
#   WEBPULSE_VAR            data dir     (default: $PREFIX/var/webpulse)
#   WEBPULSE_CRON_DIR       cron.d dir   (default: $PREFIX/etc/cron.d)
#   WEBPULSE_APPS_DIR       AppConfig dir (default: $PREFIX/var/cpanel/apps)
#   WEBPULSE_SKIP_FIRST_COLLECT=1  do not start the first collect in the background
#
# WebPulse is a single static binary. install.sh obtains the release layout
# (bin/webpulse, cpanel/, whm/, hooks/, etc/, installer/, VERSION, ...) and sets
#   SRC          directory with that layout
#   WP_BIN_SRC   the binary to install (default: $SRC/bin/webpulse)
# Nothing here needs PHP; cPanel's own PHP is only checked (informational) because
# the cPanel dashboard shims (*.live.php) run on it.

# ---------------------------------------------------------------- output ---

if [ -t 1 ] && [ -z "${NO_COLOR:-}" ] && [ "${TERM:-dumb}" != "dumb" ]; then
    C_RED=$'\033[31m'; C_GREEN=$'\033[32m'; C_YELLOW=$'\033[33m'; C_BLUE=$'\033[34m'
    C_BOLD=$'\033[1m'; C_DIM=$'\033[2m'; C_OFF=$'\033[0m'
else
    C_RED=''; C_GREEN=''; C_YELLOW=''; C_BLUE=''; C_BOLD=''; C_DIM=''; C_OFF=''
fi

wp_head() { printf '\n%s==>%s %s%s%s\n' "$C_BLUE" "$C_OFF" "$C_BOLD" "$*" "$C_OFF"; }
wp_info() { printf '    %s\n' "$*"; }
wp_ok()   { printf '    %s[ok]%s %s\n' "$C_GREEN" "$C_OFF" "$*"; }
wp_warn() { printf '    %s[warn]%s %s\n' "$C_YELLOW" "$C_OFF" "$*" >&2; }
wp_err()  { printf '%s[error]%s %s\n' "$C_RED" "$C_OFF" "$*" >&2; }
wp_dim()  { printf '    %s%s%s\n' "$C_DIM" "$*" "$C_OFF"; }

# ------------------------------------------------------------ environment ---

wp_init_env() {
    PREFIX="${WEBPULSE_PREFIX:-}"
    PREFIX="${PREFIX%/}"
    FAKE="${WEBPULSE_FAKE_CPANEL:-0}"

    INSTALL_LINK="$PREFIX/usr/local/webpulse"
    RELEASES="$PREFIX/usr/local/webpulse-releases"
    ETC_DIR="${WEBPULSE_ETC:-$PREFIX/etc/webpulse}"
    VAR_DIR="${WEBPULSE_VAR:-$PREFIX/var/webpulse}"
    CRON_DIR="${WEBPULSE_CRON_DIR:-$PREFIX/etc/cron.d}"
    APPS_DIR="${WEBPULSE_APPS_DIR:-$PREFIX/var/cpanel/apps}"
    CPANEL_ROOT="${WEBPULSE_CPANEL_ROOT:-$PREFIX/usr/local/cpanel}"
    CPANEL_ROOT="${CPANEL_ROOT%/}"

    CFG_FILE="$ETC_DIR/config.json"
    CRON_FILE="$CRON_DIR/webpulse"
    THEME_ROOT="$CPANEL_ROOT/base/frontend"
    CGI_DIR="$CPANEL_ROOT/whostmgr/docroot/cgi/webpulse"
    ADDON_ICON="$CPANEL_ROOT/whostmgr/docroot/addon_plugins/webpulse.png"
    APPCONF_FILE="$APPS_DIR/webpulse.conf"

    INSTALL_PLUGIN="$CPANEL_ROOT/scripts/install_plugin"
    UNINSTALL_PLUGIN="$CPANEL_ROOT/scripts/uninstall_plugin"
    RESTART_CPSRVD="$CPANEL_ROOT/scripts/restartsrv_cpsrvd"
    REGISTER_APPCONFIG="$CPANEL_ROOT/bin/register_appconfig"
    UNREGISTER_APPCONFIG="$CPANEL_ROOT/bin/unregister_appconfig"
    MANAGE_HOOKS="$CPANEL_ROOT/bin/manage_hooks"
    CPANEL_PHP="$CPANEL_ROOT/3rdparty/bin/php"

    HOOK_SCRIPTS=("RunAll|stats_runall_pre.sh" "RunUser|stats_runuser_pre.sh")
    DEFAULT_VAR=/var/webpulse

    WP_OUT=''
    WP_UNDO=()
    WP_ROLLING_BACK=0
    WAS_INSTALLED=0
    PREV_TARGET=''
    REL=''
    VERSION=''
    WP_THEMES=()
    WP_WARNINGS=()
}

wp_is_root() { [ "$(id -u)" = "0" ]; }

# chown root:root (recursively) when we are root; a no-op otherwise (tests)
wp_chown_root() {
    if wp_is_root; then
        chown -R root:root "$@" 2>/dev/null || true
    fi
}

# ------------------------------------------------------- command wrapper ---

# wp_fail "message": print, roll back, exit 1
wp_fail() {
    wp_err "$*"
    wp_rollback
    exit 1
}

# wp_run "description" cmd args... : run a command, capture output in WP_OUT;
# on failure print the command and its output, roll back and exit.
wp_run() {
    local desc=$1 rc=0
    shift
    WP_OUT=$("$@" 2>&1) || rc=$?
    if [ "$rc" -ne 0 ]; then
        wp_err "Step failed: $desc"
        wp_err "Command: $*"
        wp_err "Exit status: $rc"
        if [ -n "$WP_OUT" ]; then
            printf '%s\n' "$WP_OUT" | sed 's/^/    | /' >&2
        fi
        wp_rollback
        exit 1
    fi
    if [ "${WEBPULSE_VERBOSE:-0}" = "1" ] && [ -n "$WP_OUT" ]; then
        printf '%s\n' "$WP_OUT" | sed "s/^/    ${C_DIM}| /;s/\$/${C_OFF}/"
    fi
    return 0
}

# wp_run_idem "description" cmd args... : like wp_run, but "already installed/registered"
# style complaints (a re-run of an idempotent registration) are not errors.
wp_run_idem() {
    local desc=$1 rc=0
    shift
    WP_OUT=$("$@" 2>&1) || rc=$?
    if [ "$rc" -ne 0 ] && grep -qi 'already' <<<"$WP_OUT"; then
        wp_info "$desc: already in place"
        return 0
    fi
    if [ "$rc" -ne 0 ]; then
        wp_err "Step failed: $desc"
        wp_err "Command: $*"
        wp_err "Exit status: $rc"
        if [ -n "$WP_OUT" ]; then
            printf '%s\n' "$WP_OUT" | sed 's/^/    | /' >&2
        fi
        wp_rollback
        exit 1
    fi
    return 0
}

# wp_try "description" cmd args... : like wp_run but a failure is only a warning.
wp_try() {
    local desc=$1 rc=0
    shift
    WP_OUT=$("$@" 2>&1) || rc=$?
    if [ "$rc" -ne 0 ]; then
        wp_warn "$desc failed (exit $rc): $*"
        if [ -n "$WP_OUT" ]; then
            printf '%s\n' "$WP_OUT" | sed 's/^/    | /' >&2
        fi
        WP_WARNINGS+=("$desc failed")
    fi
    return "$rc"
}

# ---------------------------------------------------------------- rollback ---

# wp_undo cmd args... : remember a command that reverts the step just done.
wp_undo() {
    local entry
    entry=$(printf '%q ' "$@")
    WP_UNDO+=("$entry")
}

wp_rollback() {
    [ "$WP_ROLLING_BACK" = 1 ] && return 0
    WP_ROLLING_BACK=1
    local i
    if [ "${#WP_UNDO[@]}" -eq 0 ]; then
        return 0
    fi
    wp_warn "Rolling back the changes made by this run..."
    set +e
    for ((i = ${#WP_UNDO[@]} - 1; i >= 0; i--)); do
        eval "${WP_UNDO[$i]}" >/dev/null 2>&1
    done
    set -e
    wp_warn "Rollback finished. The system is back to its previous state."
    return 0
}

# ----------------------------------------------------------- small helpers ---

wp_safe_rm_rf() {
    # refuse to delete anything that is not clearly one of our own paths
    local p
    for p in "$@"; do
        case "$p" in
            ''|/|/usr|/usr/local|/etc|/var|/home|/root|/usr/local/cpanel)
                wp_err "Refusing to remove '$p'"
                return 1
                ;;
        esac
        rm -rf -- "$p"
    done
}

wp_swap_symlink() { # target link
    local target=$1 link=$2 tmp
    tmp="$link.new.$$"
    ln -sfn "$target" "$tmp"
    mv -fT "$tmp" "$link"
}

wp_escape_sed() { printf '%s' "$1" | sed 's/[\\&|]/\\&/g'; }

# Informational: the two LiveAPI shims of the cPanel dashboard need cPanel's PHP.
wp_check_cpanel_php() {
    local php=''
    if [ -x "$CPANEL_PHP" ]; then
        php=$CPANEL_PHP
    else
        php=$(command -v php 2>/dev/null || true)
    fi
    if [ -n "$php" ]; then
        wp_info "PHP for the cPanel page shims: $php ($("$php" -r 'echo PHP_VERSION;' 2>/dev/null || echo unknown))"
    else
        wp_warn "cPanel's PHP ($CPANEL_PHP) was not found. The cPanel dashboard page runs on it; WHM and the CLI do not need PHP."
    fi
}

# ------------------------------------------------------------- preflight ---

wp_preflight_common() {
    if ! wp_is_root && [ -z "$PREFIX" ] && [ "$FAKE" != 1 ]; then
        wp_err "This installer must be run as root."
        exit 1
    fi
    local tool
    for tool in tar cp mv ln rm mkdir chmod find sed grep diff awk; do
        command -v "$tool" >/dev/null 2>&1 || { wp_err "Required tool not found: $tool"; exit 1; }
    done
}

wp_preflight_cpanel() {
    if [ ! -e "$CPANEL_ROOT/cpanel" ]; then
        wp_err "cPanel & WHM not found ($CPANEL_ROOT/cpanel is missing)."
        wp_err "Run on a cPanel server, or use --standalone to install only the CLI and the dev server."
        exit 1
    fi
    local f
    for f in "$INSTALL_PLUGIN" "$UNINSTALL_PLUGIN" "$REGISTER_APPCONFIG" "$UNREGISTER_APPCONFIG" "$MANAGE_HOOKS"; do
        if [ ! -x "$f" ]; then
            wp_err "Required cPanel tool is missing or not executable: $f"
            exit 1
        fi
    done
}

# --------------------------------------------------------------- staging ---

# Copies the allow-listed parts of $SRC into $1 (a fresh directory). The binary is
# installed separately (wp_stage_release), whatever $SRC/bin contains.
wp_copy_tree() {
    local dest=$1 item
    mkdir -p "$dest"
    for item in cpanel whm hooks etc installer VERSION LICENSE README.md CHANGELOG.md install.sh uninstall.sh; do
        if [ -e "$SRC/$item" ]; then
            cp -a "$SRC/$item" "$dest/$item"
        fi
    done
}

wp_stage_release() {
    wp_head "Staging release"
    local bin_src="${WP_BIN_SRC:-$SRC/bin/webpulse}"
    [ -f "$bin_src" ] || wp_fail "No WebPulse binary found ($bin_src does not exist)"
    [ -s "$bin_src" ] || wp_fail "The WebPulse binary $bin_src is empty"
    [ -f "$SRC/VERSION" ] || wp_fail "Source $SRC does not contain a VERSION file"
    VERSION=$(tr -d ' \t\r\n' <"$SRC/VERSION")
    case "$VERSION" in
        ''|*[!0-9A-Za-z._+-]*) wp_fail "Invalid version string '$VERSION'" ;;
    esac

    mkdir -p "$RELEASES"
    local stage="$RELEASES/.stage.$$"
    wp_safe_rm_rf "$stage"
    wp_undo wp_safe_rm_rf "$stage"
    wp_copy_tree "$stage"
    mkdir -p "$stage/bin"
    cp -f "$bin_src" "$stage/bin/webpulse"

    find "$stage" -type d -exec chmod 755 {} +
    find "$stage" -type f -exec chmod go+r,go-w {} +
    chmod 755 "$stage/bin/webpulse"
    local f
    for f in "$stage"/hooks/*.sh "$stage"/whm/cgi/*.cgi "$stage"/install.sh "$stage"/uninstall.sh; do
        [ -f "$f" ] && chmod 755 "$f"
    done
    wp_chown_root "$stage"

    # does the binary run on this machine (architecture, libc, noexec mount)?
    local bin_version
    if ! bin_version=$("$stage/bin/webpulse" --version 2>&1); then
        wp_err "The WebPulse binary does not run on this machine:"
        printf '%s\n' "$bin_version" | sed 's/^/    | /' >&2
        wp_err "It must be a Linux $(uname -m) build (see docs/INSTALL.md, 'Building from source')."
        wp_rollback
        exit 1
    fi

    REL="$RELEASES/$VERSION"
    if [ -d "$REL" ] && diff -rq "$stage" "$REL" >/dev/null 2>&1; then
        wp_safe_rm_rf "$stage"
        wp_ok "Release $VERSION is already staged ($REL)"
    else
        if [ -e "$REL" ]; then
            REL="$RELEASES/$VERSION-$(date +%Y%m%d%H%M%S)"
        fi
        mv "$stage" "$REL"
        wp_undo wp_safe_rm_rf "$REL"
        wp_ok "Release staged in $REL"
    fi
    wp_info "Binary:  $(printf '%s' "$bin_version" | head -n 1)"
    if [ "${WP_STANDALONE:-0}" != 1 ]; then
        wp_check_cpanel_php
    fi
}

# `webpulse setup` writes a pristine config itself. When the installer is
# redirected (WEBPULSE_PREFIX / WEBPULSE_VAR) the paths inside must follow, so the
# shipped default is rewritten first: absolute strings inside the "paths" object and
# the GeoIP database paths get the prefix (/var/webpulse follows WEBPULSE_VAR),
# cron_file points at the redirected cron file; placeholders such as {user} stay.
wp_seed_config() {
    [ -f "$CFG_FILE" ] && return 0
    [ -z "$PREFIX" ] && [ "$VAR_DIR" = "$DEFAULT_VAR" ] && return 0
    local default="$REL/etc/config.json.default"
    [ -f "$default" ] || return 0
    mkdir -p "$ETC_DIR"
    wp_undo rm -f "$CFG_FILE"
    WP_VAR_DIR="$VAR_DIR" WP_PREFIX="$PREFIX" WP_CRON_FILE="$CRON_FILE" awk '
function rewrite(line,    out, tok) {
    out = ""
    while (match(line, /"\/[^"]*"/)) {
        tok = substr(line, RSTART + 1, RLENGTH - 2)
        if (tok == "/var/webpulse" || index(tok, "/var/webpulse/") == 1) {
            tok = ENVIRON["WP_VAR_DIR"] substr(tok, 14)
        } else {
            tok = ENVIRON["WP_PREFIX"] tok
        }
        out = out substr(line, 1, RSTART - 1) "\"" tok "\""
        line = substr(line, RSTART + RLENGTH)
    }
    return out line
}
/"paths"[ \t]*:[ \t]*\{/ { inpaths = 1; print; next }
inpaths && /^[ \t]*\}/ { inpaths = 0; print; next }
inpaths && /"cron_file"/ { print "    \"cron_file\": \"" ENVIRON["WP_CRON_FILE"] "\"" (($0 ~ /,[ \t]*$/) ? "," : ""); next }
inpaths { print rewrite($0); next }
/"country_db"|"city_db"/ { print rewrite($0); next }
{ print }
' "$default" >"$CFG_FILE" || wp_fail "Cannot seed $CFG_FILE from $default"
    chmod 600 "$CFG_FILE"
}

wp_run_setup() {
    wp_head "Initialising configuration and data directories"
    wp_seed_config
    export WEBPULSE_CONFIG="$CFG_FILE"
    wp_run "bin/webpulse setup" "$REL/bin/webpulse" setup
    wp_ok "webpulse setup completed (config: $CFG_FILE, data: $VAR_DIR)"
}

wp_activate_release() {
    wp_head "Activating release"
    mkdir -p "$(dirname "$INSTALL_LINK")"
    if [ -L "$INSTALL_LINK" ]; then
        PREV_TARGET=$(readlink "$INSTALL_LINK")
    elif [ -d "$INSTALL_LINK" ]; then
        # a plain directory from a manual install: park it as a release
        local legacy
        legacy="$RELEASES/legacy-$(date +%Y%m%d%H%M%S)"
        mv "$INSTALL_LINK" "$legacy"
        PREV_TARGET=$legacy
        wp_info "Moved the existing directory $INSTALL_LINK to $legacy"
    fi
    wp_swap_symlink "$REL" "$INSTALL_LINK"
    if [ -n "$PREV_TARGET" ]; then
        wp_undo wp_swap_symlink "$PREV_TARGET" "$INSTALL_LINK"
    else
        wp_undo rm -f "$INSTALL_LINK"
    fi
    wp_ok "$INSTALL_LINK -> $REL"
}

# --------------------------------------------------------------- themes ---

# Default: every real cPanel theme on the server, i.e. a directory under base/frontend/
# that ships a dynamicui.conf (Jupiter, Meridian, ...) plus the well-known names when
# present. --all-themes takes every directory, --theme adds names explicitly.
wp_select_themes() {
    local t name
    WP_THEMES=()
    if [ -d "$THEME_ROOT" ]; then
        for t in "$THEME_ROOT"/*/; do
            [ -d "$t" ] || continue
            name=$(basename "$t")
            case "$name" in ''|.*|*[!A-Za-z0-9._-]*) continue ;; esac
            if [ "${WP_ALL_THEMES:-0}" = 1 ] || [ -f "$t/dynamicui.conf" ] || [ "$name" = jupiter ] || [ "$name" = meridian ]; then
                WP_THEMES+=("$name")
            fi
        done
    fi
    [ "${#WP_THEMES[@]}" -gt 0 ] || WP_THEMES+=("jupiter")
    for t in ${WP_EXTRA_THEMES[@]+"${WP_EXTRA_THEMES[@]}"}; do
        case " ${WP_THEMES[*]} " in *" $t "*) ;; *) WP_THEMES+=("$t") ;; esac
    done
}

# Copies the account-side files of release $1 into theme directory $2.
wp_copy_theme_files() {
    local rel=$1 theme=$2
    local base="$THEME_ROOT/$theme" dest stage
    dest="$base/webpulse"
    stage="$base/.webpulse.new.$$"
    wp_safe_rm_rf "$stage"
    mkdir -p "$stage"
    # thin PHP shims (index.live.php, api.live.php, lib/*.php) + icon; no PHP library any more
    cp -a "$rel/cpanel/webpulse/." "$stage/"
    cp -a "$rel/VERSION" "$stage/VERSION"
    find "$stage" -type d -exec chmod 755 {} +
    find "$stage" -type f -exec chmod 644 {} +
    wp_chown_root "$stage"
    wp_safe_rm_rf "$dest"
    mv "$stage" "$dest"
}

wp_copy_whm_files() {
    local rel=$1
    mkdir -p "$CGI_DIR" "$(dirname "$ADDON_ICON")"
    local f
    for f in "$rel"/whm/cgi/*.cgi; do
        cp -f "$f" "$CGI_DIR/$(basename "$f")"
        chmod 755 "$CGI_DIR/$(basename "$f")"
    done
    cp -f "$rel/whm/addon_plugins/webpulse.png" "$ADDON_ICON"
    chmod 644 "$ADDON_ICON"
    wp_chown_root "$CGI_DIR" "$ADDON_ICON"
}

wp_resync_frontends() { # undo helper used by update rollbacks
    local rel=$1 t
    [ -d "$rel" ] || return 0
    for t in "${WP_THEMES[@]}"; do
        [ -d "$THEME_ROOT/$t" ] && wp_copy_theme_files "$rel" "$t"
    done
    wp_copy_whm_files "$rel"
}

wp_install_cpanel_plugin() {
    wp_head "cPanel plugin (account dashboard)"
    wp_select_themes
    local t installed=0
    for t in "${WP_THEMES[@]}"; do
        if [ ! -d "$THEME_ROOT/$t" ]; then
            if [ "$t" = jupiter ]; then
                wp_fail "Theme directory $THEME_ROOT/$t not found. Is this a supported cPanel version (Jupiter or Meridian theme)?"
            fi
            wp_warn "Theme '$t' not found under $THEME_ROOT, skipped"
            continue
        fi
        local existed=0
        [ -d "$THEME_ROOT/$t/webpulse" ] && existed=1
        if [ "$WAS_INSTALLED" = 1 ] && [ -n "$PREV_TARGET" ]; then
            wp_undo wp_resync_frontends "$PREV_TARGET"
        elif [ "$existed" = 0 ]; then
            wp_undo wp_safe_rm_rf "$THEME_ROOT/$t/webpulse"
        fi
        wp_copy_theme_files "$REL" "$t"
        wp_ok "Plugin files copied to $THEME_ROOT/$t/webpulse"
        wp_run_idem "install_plugin ($t)" "$INSTALL_PLUGIN" "$INSTALL_LINK/cpanel/plugin" --theme "$t"
        [ "$WAS_INSTALLED" = 0 ] && wp_undo wp_try_quiet "$UNINSTALL_PLUGIN" "$INSTALL_LINK/cpanel/plugin" --theme "$t"
        wp_ok "Registered with cPanel ($t theme, Metrics group)"
        installed=$((installed + 1))
    done
    [ "$installed" -gt 0 ] || wp_fail "No cPanel theme could be installed"
}

wp_try_quiet() { "$@" >/dev/null 2>&1 || true; }

# ------------------------------------------------------------------ WHM ---

wp_install_whm_plugin() {
    wp_head "WHM plugin (administrator dashboard)"
    if [ "$WAS_INSTALLED" = 0 ]; then
        wp_undo wp_safe_rm_rf "$CGI_DIR"
        wp_undo rm -f "$ADDON_ICON"
    fi
    wp_copy_whm_files "$REL"
    wp_ok "CGI scripts copied to $CGI_DIR"
    if [ "$WAS_INSTALLED" = 0 ]; then
        wp_undo wp_try_quiet "$UNREGISTER_APPCONFIG" webpulse
    fi
    wp_run_idem "register_appconfig" "$REGISTER_APPCONFIG" "$INSTALL_LINK/whm/webpulse.conf"
    wp_ok "AppConfig registered (WHM > Plugins > WebPulse Analytics)"
    # the docs do not say when a restart is needed: only do it when the tool asks for it
    if grep -qi 'restart' <<<"$WP_OUT"; then
        wp_restart_cpsrvd
    fi
}

wp_restart_cpsrvd() {
    if [ -x "$RESTART_CPSRVD" ]; then
        wp_try "restartsrv_cpsrvd" "$RESTART_CPSRVD" && wp_ok "cpsrvd restarted"
    else
        wp_warn "cpsrvd should be restarted ($RESTART_CPSRVD not found)"
    fi
}

# ---------------------------------------------------------------- hooks ---

wp_hook_registered() { # path
    "$MANAGE_HOOKS" list 2>/dev/null | grep -Fq -- "$1"
}

wp_register_hooks() {
    wp_head "cPanel hooks (drain logs before cpanellogd)"
    local spec event script path rc
    for spec in "${HOOK_SCRIPTS[@]}"; do
        event=${spec%%|*}
        script=${spec#*|}
        path="$INSTALL_LINK/hooks/$script"
        if wp_hook_registered "$path"; then
            wp_ok "Stats::$event hook already registered"
            continue
        fi
        rc=0
        WP_OUT=$("$MANAGE_HOOKS" add script "$path" --manual --category Stats --event "$event" --stage pre 2>&1) || rc=$?
        if [ "$rc" -ne 0 ] && ! grep -qi 'already' <<<"$WP_OUT"; then
            wp_err "Step failed: register Stats::$event hook"
            wp_err "Command: $MANAGE_HOOKS add script $path --manual --category Stats --event $event --stage pre"
            wp_err "Exit status: $rc"
            printf '%s\n' "$WP_OUT" | sed 's/^/    | /' >&2
            wp_rollback
            exit 1
        fi
        [ "$WAS_INSTALLED" = 0 ] && wp_undo wp_try_quiet "$MANAGE_HOOKS" delete script "$path" --manual --category Stats --event "$event" --stage pre
        wp_ok "Stats::$event (pre) hook registered"
    done

    # flag directory used by the unprivileged Stats::RunUser hook
    mkdir -p "$VAR_DIR/run/hook"
    chmod 1777 "$VAR_DIR/run/hook"
    chmod 0711 "$VAR_DIR/run"
    chmod o+x "$VAR_DIR"
    wp_ok "Hook flag directory $VAR_DIR/run/hook (mode 1777)"
}

# ----------------------------------------------------------------- cron ---

wp_install_cron() {
    wp_head "Cron"
    local tmp content_ok=0
    mkdir -p "$CRON_DIR"
    tmp="$CRON_FILE.new.$$"
    sed "s|/usr/local/webpulse|$(wp_escape_sed "$INSTALL_LINK")|g" "$REL/etc/cron.d/webpulse" >"$tmp"
    if [ -f "$CRON_FILE" ] && cmp -s "$tmp" "$CRON_FILE"; then
        content_ok=1
    fi
    if [ "$content_ok" = 1 ]; then
        rm -f "$tmp"
        wp_ok "$CRON_FILE is up to date"
        return 0
    fi
    if [ -f "$CRON_FILE" ]; then
        wp_undo cp -f "$CRON_FILE" "$CRON_FILE.rollback.$$"
        cp -f "$CRON_FILE" "$CRON_FILE.rollback.$$"
        wp_undo mv -f "$CRON_FILE.rollback.$$" "$CRON_FILE"
    else
        wp_undo rm -f "$CRON_FILE"
    fi
    chmod 644 "$tmp"
    wp_chown_root "$tmp"
    mv -f "$tmp" "$CRON_FILE"
    wp_ok "Installed $CRON_FILE (collect every 5 min, hooked every minute, rollup 02:10)"
}

# ---------------------------------------------------------------- geoip ---

wp_setup_geoip() {
    wp_head "GeoIP"
    wp_run "enable GeoIP (webpulse config set geoip.enabled true)" \
        "$REL/bin/webpulse" config set geoip.enabled true
    wp_ok "geoip.enabled = true (stored in the registry; config file: $CFG_FILE)"
    if wp_try "GeoIP database download" "$REL/bin/webpulse" geoip-update; then
        wp_ok "DB-IP country database downloaded"
    else
        wp_warn "Retry later with: $INSTALL_LINK/bin/webpulse geoip-update"
    fi
}

wp_first_collect() {
    wp_head "First collection"
    if [ "${WEBPULSE_SKIP_FIRST_COLLECT:-0}" = 1 ]; then
        wp_info "Skipped (WEBPULSE_SKIP_FIRST_COLLECT=1)"
        return 0
    fi
    ( WEBPULSE_CONFIG="$CFG_FILE" nohup "$INSTALL_LINK/bin/webpulse" collect --quiet </dev/null >/dev/null 2>&1 & )
    wp_ok "collect started in the background (progress: $INSTALL_LINK/bin/webpulse status)"
}

# --------------------------------------------------------------- pruning ---

wp_prune_releases() {
    local keep=3 n=0 d current
    [ -d "$RELEASES" ] || return 0
    current=$(readlink "$INSTALL_LINK" 2>/dev/null || true)
    # newest first
    while IFS= read -r d; do
        [ -n "$d" ] || continue
        n=$((n + 1))
        if [ "$n" -le "$keep" ] || [ "$d" = "$current" ] || [ "$d" = "$PREV_TARGET" ]; then
            continue
        fi
        wp_safe_rm_rf "$d"
        wp_info "Removed old release $d"
    done < <(find "$RELEASES" -mindepth 1 -maxdepth 1 -type d ! -name '.stage.*' -printf '%T@ %p\n' 2>/dev/null | sort -rn | cut -d' ' -f2-)
}

# ------------------------------------------------------------- summary ---

wp_hostname() { hostname -f 2>/dev/null || hostname 2>/dev/null || echo localhost; }

wp_summary_install() {
    local host
    host=$(wp_hostname)
    printf '\n%s%sWebPulse %s is installed.%s\n\n' "$C_GREEN" "$C_BOLD" "$VERSION" "$C_OFF"
    if [ "${WP_STANDALONE:-0}" = 1 ]; then
        wp_info "Standalone mode: CLI and dev server only."
        wp_info "Try:  $INSTALL_LINK/bin/webpulse status"
        wp_info "      $INSTALL_LINK/bin/webpulse serve --port 8099 --user <account>"
    else
        wp_info "WHM (administrator):  https://$host:2087/cgi/webpulse/index.cgi"
        wp_info "                      or WHM > Plugins > WebPulse Analytics"
        wp_info "cPanel (account):     https://$host:2083/ > Metrics > WebPulse Analytics"
        wp_info "                      (grant the 'webpulse' feature in WHM > Feature Manager if you use feature lists)"
        wp_info "Command line:         $INSTALL_LINK/bin/webpulse status"
        if [ "${WP_NO_CRON:-0}" = 1 ]; then
            wp_warn "--no-cron: nothing collects data until you schedule '$INSTALL_LINK/bin/webpulse collect' yourself."
        fi
    fi
    wp_info "Config:               $CFG_FILE"
    wp_info "Data:                 $VAR_DIR"
    wp_info "Uninstall:            $INSTALL_LINK/uninstall.sh   (add --purge to delete the statistics too)"
    if [ "${#WP_WARNINGS[@]}" -gt 0 ]; then
        printf '\n'
        wp_warn "Completed with ${#WP_WARNINGS[@]} warning(s): $(IFS=';'; echo "${WP_WARNINGS[*]}")"
    fi
    printf '\n'
}

# --------------------------------------------------------------- confirm ---

wp_confirm() { # prompt
    if [ "${WP_YES:-0}" = 1 ] || [ ! -t 0 ] || [ ! -t 1 ]; then
        return 0
    fi
    local ans=''
    printf '%s [Y/n] ' "$1"
    read -r ans || ans=n
    case "$ans" in n|N|no|NO|No) return 1 ;; esac
    return 0
}

# ----------------------------------------------------------- the install ---

wp_install() {
    wp_init_env
    wp_preflight_common
    [ "${WP_STANDALONE:-0}" = 1 ] || wp_preflight_cpanel

    if [ -e "$INSTALL_LINK/bin/webpulse" ]; then
        WAS_INSTALLED=1
        PREV_TARGET=$(readlink "$INSTALL_LINK" 2>/dev/null || true)
    fi

    printf '\n%sWebPulse installer%s\n' "$C_BOLD" "$C_OFF"
    if [ "$WAS_INSTALLED" = 1 ]; then
        wp_info "Existing installation found: updating in place (data and config are kept)."
    elif [ "${WP_UPDATE:-0}" = 1 ]; then
        wp_warn "--update given but WebPulse is not installed; performing a fresh installation."
    fi
    wp_info "Source:  ${WP_SOURCE_DESC:-$SRC}"
    wp_info "Target:  $INSTALL_LINK (releases in $RELEASES)"
    wp_info "Mode:    $([ "${WP_STANDALONE:-0}" = 1 ] && echo 'standalone (CLI + dev server only)' || echo 'cPanel/WHM')"
    wp_confirm "Proceed?" || { wp_info "Aborted."; exit 0; }

    wp_stage_release
    wp_run_setup
    wp_activate_release
    if [ "${WP_STANDALONE:-0}" != 1 ]; then
        wp_install_cpanel_plugin
        wp_install_whm_plugin
        wp_register_hooks
        if [ "${WP_NO_CRON:-0}" = 1 ]; then
            wp_head "Cron"
            wp_info "Skipped (--no-cron)"
        else
            wp_install_cron
        fi
    fi
    [ "${WP_GEOIP:-0}" = 1 ] && wp_setup_geoip
    wp_prune_releases
    wp_first_collect
    wp_summary_install
}

# --------------------------------------------------------- the uninstall ---

wp_uninstall() {
    wp_init_env
    wp_preflight_common

    printf '\n%sWebPulse uninstaller%s\n' "$C_BOLD" "$C_OFF"
    wp_info "Removes the plugins, hooks, cron and code; statistics in $VAR_DIR $([ "${WP_PURGE:-0}" = 1 ] && echo 'WILL BE DELETED (--purge)' || echo 'are kept (use --purge to delete)')"
    wp_confirm "Proceed?" || { wp_info "Aborted."; exit 0; }
    if [ "${WP_PURGE:-0}" = 1 ] && [ "${WP_YES:-0}" != 1 ] && [ -t 0 ] && [ -t 1 ]; then
        local ans=''
        printf '%sThis permanently deletes all collected statistics. Type "delete" to continue:%s ' "$C_RED" "$C_OFF"
        read -r ans || ans=''
        [ "$ans" = delete ] || { wp_info "Aborted."; exit 0; }
    fi

    local have_cpanel=0
    [ -e "$CPANEL_ROOT/cpanel" ] && have_cpanel=1
    local need_restart=0 spec event script path t d

    if [ "$have_cpanel" = 1 ]; then
        wp_head "Hooks"
        if [ -x "$MANAGE_HOOKS" ]; then
            for spec in "${HOOK_SCRIPTS[@]}"; do
                event=${spec%%|*}
                script=${spec#*|}
                path="$INSTALL_LINK/hooks/$script"
                if wp_hook_registered "$path"; then
                    wp_try "Remove Stats::$event hook" "$MANAGE_HOOKS" delete script "$path" --manual --category Stats --event "$event" --stage pre \
                        && wp_ok "Stats::$event hook removed"
                else
                    wp_info "Stats::$event hook not registered"
                fi
            done
        else
            wp_warn "$MANAGE_HOOKS not found, hooks left as they are"
        fi

        wp_head "WHM plugin"
        if [ -x "$UNREGISTER_APPCONFIG" ] && { [ -f "$APPCONF_FILE" ] || [ "${WP_FORCE_UNREGISTER:-0}" = 1 ]; }; then
            wp_try "unregister_appconfig" "$UNREGISTER_APPCONFIG" webpulse && wp_ok "AppConfig unregistered"
            need_restart=1
        else
            wp_info "AppConfig not registered"
        fi
        wp_safe_rm_rf "$CGI_DIR"
        rm -f "$ADDON_ICON"
        wp_ok "CGI scripts and icon removed"

        wp_head "cPanel plugin"
        if [ -d "$THEME_ROOT" ]; then
            for d in "$THEME_ROOT"/*/; do
                [ -d "${d}webpulse" ] || continue
                t=$(basename "$d")
                if [ -x "$UNINSTALL_PLUGIN" ] && [ -d "$INSTALL_LINK/cpanel/plugin" ]; then
                    wp_try "uninstall_plugin ($t)" "$UNINSTALL_PLUGIN" "$INSTALL_LINK/cpanel/plugin" --theme "$t" \
                        && wp_ok "Unregistered from the $t theme"
                else
                    wp_warn "uninstall_plugin or $INSTALL_LINK/cpanel/plugin missing, only removing files for $t"
                fi
                wp_safe_rm_rf "${d}webpulse"
            done
        fi
        if [ "$need_restart" = 1 ]; then
            wp_restart_cpsrvd
        fi
    fi

    wp_head "Cron"
    if [ -f "$CRON_FILE" ]; then
        rm -f "$CRON_FILE"
        wp_ok "Removed $CRON_FILE"
    else
        wp_info "No cron file"
    fi

    wp_head "Code"
    if [ -L "$INSTALL_LINK" ] || [ -e "$INSTALL_LINK" ]; then
        rm -rf "$INSTALL_LINK"
        wp_ok "Removed $INSTALL_LINK"
    fi
    if [ -d "$RELEASES" ]; then
        wp_safe_rm_rf "$RELEASES"
        wp_ok "Removed $RELEASES"
    fi

    wp_head "Data"
    if [ "${WP_PURGE:-0}" = 1 ]; then
        wp_safe_rm_rf "$VAR_DIR" "$ETC_DIR"
        wp_ok "Deleted $VAR_DIR and $ETC_DIR"
    else
        rm -rf "$VAR_DIR/run/hook" 2>/dev/null || true
        wp_info "Kept $VAR_DIR and $ETC_DIR (re-install picks them up; add --purge to delete)"
    fi

    printf '\n%s%sWebPulse has been uninstalled.%s\n' "$C_GREEN" "$C_BOLD" "$C_OFF"
    if [ "${#WP_WARNINGS[@]}" -gt 0 ]; then
        wp_warn "Completed with ${#WP_WARNINGS[@]} warning(s): $(IFS=';'; echo "${WP_WARNINGS[*]}")"
    fi
    printf '\n'
}
