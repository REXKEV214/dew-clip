pid=$1
mount=$2
source=$3
target=$4
stage=$5
backup=$6
open_app=$7

clean_mount() {
    [ -n "$mount" ] || return 0
    if [ -d "$mount" ] && ! /bin/rmdir "$mount" 2>/dev/null; then
        /usr/bin/hdiutil detach -quiet "$mount" ||
            /usr/bin/hdiutil detach -force -quiet "$mount" || return 1
        if [ -d "$mount" ]; then
            /bin/rmdir "$mount" || return 1
        fi
    fi
    /bin/rm -f "${mount%/*}/image.dmg" && /bin/rmdir "${mount%/*}" || return 1
    mount=
}

trap 'clean_mount' EXIT
deadline=$((SECONDS + 15))
while kill -0 "$pid" 2>/dev/null; do
    if [ "$SECONDS" -ge "$deadline" ]; then
        exit 1
    fi
    /bin/sleep 0.1
done

staged=
backed_up=
replaced=
candidate="$stage/DewClip.app"
restore() {
    [ -n "$backed_up" ] || return 0
    if [ -n "$replaced" ]; then
        /bin/mv -h "$target" "$candidate" || return 1
        replaced=
    fi
    [ ! -e "$target" ] && [ ! -L "$target" ] &&
        /bin/mv -h "$backup" "$target" || return 1
    backed_up=
}

cleanup() {
    restore
    if [ -n "$staged" ]; then
        /bin/rm -rf "$stage"
    fi
    clean_mount
}
trap 'cleanup' EXIT
trap 'exit 1' HUP INT TERM

if /bin/mkdir -m 700 "$stage"; then
    staged=yes
    if /usr/bin/ditto "$source" "$candidate" &&
        [ ! -e "$backup" ] && [ ! -L "$backup" ] &&
        /bin/mv -h "$target" "$backup"; then
        backed_up=yes
        if [ ! -e "$target" ] && [ ! -L "$target" ] &&
            /bin/mv -h "$candidate" "$target"; then
            replaced=yes
            clean_mount
            if "$open_app" -n "$target"; then
                backed_up=
                /bin/rm -rf "$backup" || exit 1
                exit 0
            fi
        fi
    fi
fi
clean_mount
if restore && [ -d "$target" ]; then
    "$open_app" -n "$target"
fi
exit 1
