#!/bin/sh
set -eu

cd "$(dirname "$0")/.."
rt=$(mktemp -d)
chmod 700 "$rt"
kb_pid=""
sway_pid=""
seatd_pid=""

cleanup() {
    if [ -n "$sway_pid" ]; then
        kill "$sway_pid" 2>/dev/null || true
    fi
    if [ -n "$kb_pid" ]; then
        kill "$kb_pid" 2>/dev/null || true
    fi
    if [ -n "$seatd_pid" ]; then
        sudo kill "$seatd_pid" 2>/dev/null || true
    fi
    rm -rf "$rt"
}
trap cleanup EXIT

pkg() {
    spec=$1
    bin=$2
    nix build --no-link --print-out-paths "$spec" | while read -r path; do
        if [ -x "$path/bin/$bin" ]; then
            printf '%s\n' "$path"
            break
        fi
    done
}

sway=$(pkg nixpkgs/nixos-unstable#sway sway)
seatd=$(pkg nixpkgs/nixos-unstable#seatd seatd)
[ -n "$sway" ] && [ -n "$seatd" ]

sudo modprobe uinput || true
sudo chmod a+rw /dev/uinput
python3 - "$rt/keyboard" << 'PY' &
import fcntl, glob, os, struct, sys, time
ready = sys.argv[1]
fd = os.open("/dev/uinput", os.O_WRONLY | os.O_NONBLOCK)
fcntl.ioctl(fd, 0x40045564, 0)
fcntl.ioctl(fd, 0x40045564, 1)
for key in range(1, 128):
    fcntl.ioctl(fd, 0x40045565, key)
fcntl.ioctl(fd, 0x405c5503, struct.pack("HHHH80sI", 3, 1, 1, 1, b"wl-cast-ci-keyboard", 0))
fcntl.ioctl(fd, 0x5501)
found = False
deadline = time.time() + 5
while time.time() < deadline and not found:
    for path in glob.glob("/sys/class/input/event*/device/name"):
        with open(path, errors="ignore") as name:
            if "wl-cast-ci-keyboard" in name.read():
                found = True
    time.sleep(0.1)
if not found:
    sys.exit(1)
os.close(os.open(ready, os.O_CREAT | os.O_WRONLY, 0o644))
while True:
    time.sleep(3600)
PY
kb_pid=$!
i=0
while [ ! -f "$rt/keyboard" ]; do
    i=$((i + 1))
    if [ "$i" -ge 80 ]; then
        echo "virtual keyboard did not appear"
        exit 1
    fi
    sleep 0.1
done

sudo "$seatd/bin/seatd" -g "$(id -gn)" >"$rt/seatd.log" 2>&1 &
seatd_pid=$!
i=0
while [ ! -S /run/seatd.sock ]; do
    i=$((i + 1))
    if [ "$i" -ge 50 ]; then
        echo "seatd did not start"
        cat "$rt/seatd.log"
        exit 1
    fi
    sleep 0.1
done

printf 'default_border none\n' >"$rt/sway.conf"
unset WAYLAND_DISPLAY || true
export XDG_RUNTIME_DIR=$rt
export LIBSEAT_BACKEND=seatd
export WLR_BACKENDS=headless,libinput
export WLR_HEADLESS_OUTPUTS=1
export WLR_RENDERER=pixman
export WLR_RENDERER_ALLOW_SOFTWARE=1
export SWAYSOCK=$rt/sway.sock
"$sway/bin/sway" -c "$rt/sway.conf" >"$rt/sway.log" 2>&1 &
sway_pid=$!
i=0
while [ ! -S "$rt/wayland-1" ]; do
    i=$((i + 1))
    if [ "$i" -ge 50 ]; then
        echo "sway did not start"
        cat "$rt/sway.log"
        exit 1
    fi
    sleep 0.1
done
export WAYLAND_DISPLAY=wayland-1
export PATH="$sway/bin:$PATH"

if ! cargo test --test compositor -- --ignored --test-threads=1; then
    echo "--- sway"
    cat "$rt/sway.log"
    echo "--- seatd"
    cat "$rt/seatd.log"
    exit 1
fi
