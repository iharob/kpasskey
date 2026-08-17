#!/usr/bin/env bash
#
# One-off: removes every kfingerprint artefact this machine picked up from
# `make install` and the manual spike steps, so the tree can be reinstalled
# under the kpasskey name. Delete this script once the rename has landed.
#
# Run as your normal user (it needs the user systemd bus); root parts escalate
# with sudo.
#
#   ./uninstall-kfingerprint.sh --dry-run    # show what would go
#   ./uninstall-kfingerprint.sh              # do it, with a confirmation
#   ./uninstall-kfingerprint.sh --yes --purge-data --android

set -euo pipefail

PREFIX=${PREFIX:-/usr}
DRY_RUN=0
ASSUME_YES=0
PURGE_DATA=0
DO_ANDROID=0
ANDROID_PKG=org.kfingerprint

BACKUP_DIR="${XDG_STATE_HOME:-$HOME/.local/state}/kfingerprint-uninstall-$(date +%Y%m%d-%H%M%S)"

usage() {
    cat <<'EOF'
usage: uninstall-kfingerprint.sh [--dry-run] [--yes] [--purge-data] [--android]

  --dry-run     list every action, change nothing
  --yes         skip the confirmation prompt
  --purge-data  also delete paired-device records (~/.local/share/kfingerprint).
                Without this they are kept, and you can move them to the
                kpasskey directory afterwards to avoid re-pairing.
  --android     also `adb uninstall org.kfingerprint` from a connected device
EOF
}

while [ $# -gt 0 ]; do
    case "$1" in
        --dry-run)    DRY_RUN=1 ;;
        --yes|-y)     ASSUME_YES=1 ;;
        --purge-data) PURGE_DATA=1 ;;
        --android)    DO_ANDROID=1 ;;
        -h|--help)    usage; exit 0 ;;
        *)            echo "unknown option: $1" >&2; usage >&2; exit 2 ;;
    esac
    shift
done

if [ "$(id -u)" -eq 0 ]; then
    echo "run this as your normal user, not root: systemctl --user needs your session bus" >&2
    exit 1
fi

say()  { printf '  %s\n' "$*"; }
step() { printf '\n\033[1m%s\033[0m\n' "$*"; }

run() {
    if [ "$DRY_RUN" -eq 1 ]; then
        say "would run: $*"
    else
        "$@"
    fi
}

# Files installed by `make install` (Makefile: install-daemon/install-pam/install-kcm),
# plus pam_kfp_stub.so, which was copied in by hand during the spike.
FILES=(
    "$PREFIX/bin/kfp-spike"
    "$PREFIX/lib/security/pam_kfp_spike.so"
    "$PREFIX/lib/security/pam_kfp_stub.so"
    "$PREFIX/lib/systemd/system/kfingerprint-fido.service"
    "$PREFIX/lib/systemd/user/kfingerprint.service"
    "$PREFIX/lib/tmpfiles.d/kfingerprint.conf"
    "$PREFIX/share/kfingerprint/polkit-1.example"
    "$PREFIX/lib/qt6/plugins/plasma/kcms/systemsettings/kcm_kfingerprint.so"
    "$PREFIX/share/applications/kcm_kfingerprint.desktop"
)

# The KCM's own install manifest is authoritative for where CMake actually put it,
# which beats guessing the Qt plugin path on a machine with a different Qt layout.
MANIFEST="$(dirname "$0")/kcm/build/install_manifest.txt"
if [ -r "$MANIFEST" ]; then
    while IFS= read -r line; do
        [ -n "$line" ] || continue
        for known in "${FILES[@]}"; do
            [ "$known" = "$line" ] && continue 2
        done
        FILES+=("$line")
    done < "$MANIFEST"
fi

DIRS=(
    "$PREFIX/share/kfingerprint"
    /run/kfingerprint
)

# /etc/pam.d entries the spike wrote. Removing one is only safe because pam 1.5+
# falls back to /usr/lib/pam.d/<service> — which is where polkit and kscreenlocker
# ship the originals. That fallback is verified per file before anything is deleted;
# without it the service would drop through to /etc/pam.d/other, which denies all.
PAM_SERVICES=(polkit-1 kde-fingerprint)

pam_removable() {
    local svc="$1"
    [ -f "/etc/pam.d/$svc" ] || return 1
    grep -q 'pam_kfp' "/etc/pam.d/$svc" || return 1
    [ -f "/usr/lib/pam.d/$svc" ] || return 1
}

step "1. Plan"

PRESENT_FILES=()
for f in "${FILES[@]}"; do
    [ -e "$f" ] && PRESENT_FILES+=("$f")
done
PRESENT_PAM=()
PAM_SKIPPED=()
for svc in "${PAM_SERVICES[@]}"; do
    if pam_removable "$svc"; then
        PRESENT_PAM+=("$svc")
    elif [ -f "/etc/pam.d/$svc" ] && grep -q 'pam_kfp' "/etc/pam.d/$svc"; then
        PAM_SKIPPED+=("$svc")
    fi
done

say "files to remove:      ${#PRESENT_FILES[@]}"
for f in "${PRESENT_FILES[@]}"; do say "    $f"; done
say "pam stacks to revert: ${#PRESENT_PAM[@]}"
for svc in "${PRESENT_PAM[@]}"; do
    say "    /etc/pam.d/$svc  (falls back to /usr/lib/pam.d/$svc)"
done
for svc in "${PAM_SKIPPED[@]}"; do
    say "    /etc/pam.d/$svc  SKIPPED: no /usr/lib/pam.d/$svc to fall back to"
done
say "backups:              $BACKUP_DIR"
if [ "$PURGE_DATA" -eq 1 ]; then
    say "device records:       DELETE $HOME/.local/share/kfingerprint"
else
    say "device records:       keep $HOME/.local/share/kfingerprint (--purge-data to delete)"
fi

if [ ${#PAM_SKIPPED[@]} -gt 0 ]; then
    printf '\n\033[31mWARNING\033[0m: %s\n' \
        "the skipped stack(s) above still load pam_kfp_*.so, which this script is about to delete."
    say "Edit them by hand — with a root shell already open on another TTY — before continuing."
fi

if [ "$DRY_RUN" -eq 0 ] && [ "$ASSUME_YES" -eq 0 ]; then
    printf '\nProceed? [y/N] '
    read -r reply
    case "$reply" in
        [yY]|[yY][eE][sS]) ;;
        *) echo "aborted"; exit 0 ;;
    esac
fi

step "2. Stop and disable units"

run systemctl --user disable --now kfingerprint.service || say "user unit: already gone"
run sudo systemctl disable --now kfingerprint-fido.service || say "system unit: already gone"
if pgrep -x kfp-spike >/dev/null 2>&1; then
    run pkill -x kfp-spike || true
fi

step "3. Back up what is about to be deleted"

run mkdir -p "$BACKUP_DIR"
for svc in "${PRESENT_PAM[@]}"; do
    run sudo cp -a "/etc/pam.d/$svc" "$BACKUP_DIR/pam.d-$svc"
done
for f in "${PRESENT_FILES[@]}"; do
    case "$f" in
        /etc/*) run sudo cp -a "$f" "$BACKUP_DIR/$(echo "${f#/}" | tr / -)" ;;
    esac
done
if [ -d "$BACKUP_DIR" ]; then
    run sudo chown -R "$(id -u):$(id -g)" "$BACKUP_DIR"
fi

step "4. Revert PAM"

for svc in "${PRESENT_PAM[@]}"; do
    say "/etc/pam.d/$svc -> vendor default"
    run sudo rm -f "/etc/pam.d/$svc"
done
[ ${#PRESENT_PAM[@]} -eq 0 ] && say "nothing to revert"

step "5. Remove installed files"

for f in "${PRESENT_FILES[@]}"; do
    say "$f"
    run sudo rm -f "$f"
done
[ ${#PRESENT_FILES[@]} -eq 0 ] && say "nothing installed"

for d in "${DIRS[@]}"; do
    if [ -d "$d" ]; then
        say "$d"
        run sudo rm -rf "$d"
    fi
done

step "6. Reload"

run sudo systemctl daemon-reload
run systemctl --user daemon-reload
run sudo systemd-tmpfiles --clean || true
if command -v update-desktop-database >/dev/null 2>&1; then
    run sudo update-desktop-database "$PREFIX/share/applications" || true
fi
if command -v kbuildsycoca6 >/dev/null 2>&1; then
    run kbuildsycoca6 --noincremental >/dev/null 2>&1 || true
fi

step "7. User data"

DATA_DIR="$HOME/.local/share/kfingerprint"
if [ -d "$DATA_DIR" ]; then
    if [ "$PURGE_DATA" -eq 1 ]; then
        say "removing $DATA_DIR"
        run rm -rf "$DATA_DIR"
    else
        say "kept: $DATA_DIR"
        say "to carry pairings over: mv '$DATA_DIR' '$HOME/.local/share/kpasskey'"
    fi
else
    say "no device records"
fi

step "8. Root-owned build tree"

# A past `sudo cargo build` left ~875 root-owned files under daemon/target, which
# `make clean` cannot remove and which would shadow the renamed crates.
TARGET_DIR="$(cd "$(dirname "$0")" && pwd)/daemon/target"
if [ -d "$TARGET_DIR" ] && [ -n "$(find "$TARGET_DIR" ! -user "$(id -un)" -print -quit 2>/dev/null)" ]; then
    say "removing $TARGET_DIR (contains root-owned objects)"
    run sudo rm -rf "$TARGET_DIR"
elif [ -d "$TARGET_DIR" ]; then
    say "$TARGET_DIR is yours — 'make clean' handles it"
else
    say "no build tree"
fi

step "9. Android"

if [ "$DO_ANDROID" -eq 1 ]; then
    if ! command -v adb >/dev/null 2>&1; then
        say "adb not installed — uninstall $ANDROID_PKG from the phone by hand"
    elif [ -z "$(adb devices | awk 'NR>1 && $2=="device"')" ]; then
        say "no device attached — uninstall $ANDROID_PKG from the phone by hand"
    else
        run adb uninstall "$ANDROID_PKG" || say "$ANDROID_PKG was not installed"
    fi
else
    say "skipped (--android to uninstall $ANDROID_PKG via adb)"
fi

step "Done"
say "backups: $BACKUP_DIR"
say "next: cd /data/Code/kpasskey && make && sudo make install"
