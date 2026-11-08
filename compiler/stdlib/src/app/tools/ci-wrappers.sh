#!/bin/sh
set -eu
here=$(cd "$(dirname "$0")" && pwd)
app=$(dirname "$here")
repo=$(cd "$app/../../../.." && pwd)
if [ -n "${RNX_BIN:-}" ]; then
    rnx=$RNX_BIN
elif [ -x "$repo/target/release/rnx" ]; then
    rnx=$repo/target/release/rnx
else
    rnx=$repo/target/debug/rnx
fi
prefix=${WRAPPER_PREFIX:-$repo/.rnx-cache/wrappers}
backend=${RNX_BACKEND:-cranelift}

if [ -z "${RNX_VENDOR_DIR:-}" ]; then
    stage=${RNX_STAGE_DIR:-$repo/.rnx-cache/std-vendor}
    "$here/stage-stdlib.sh" "$stage"
    export RNX_VENDOR_DIR="$stage"
else
    export RNX_VENDOR_DIR
fi
export RNX_BIN="$rnx"

media_dir="$repo/.rnx-cache/media-ci"
media_video=""
if command -v ffmpeg >/dev/null 2>&1; then
    mkdir -p "$media_dir"
    if [ ! -s "$media_dir/clip.mp4" ]; then
        ffmpeg -y -f lavfi -i testsrc=size=160x120:rate=10 -t 1 -pix_fmt yuv420p "$media_dir/clip.mp4" >/dev/null 2>&1 || true
    fi
    if [ -s "$media_dir/clip.mp4" ]; then
        media_video="$media_dir/clip.mp4"
    fi
fi
export RNX_AUDIO=fast
export RNX_CAMERA=stub
export RNX_TEST_VIDEO="$media_video"
export RNX_PICK_FILE="$repo/website/static/apple-touch-icon.png"

if [ ! -x "$rnx" ]; then
    echo "no rnx binary at $rnx" >&2
    exit 1
fi

if [ -f /usr/share/vulkan/icd.d/lvp_icd.x86_64.json ]; then
    export VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.x86_64.json
elif [ -f /usr/share/vulkan/icd.d/lvp_icd.json ]; then
    export VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json
fi

"$here/build-wrappers.sh"

export PKG_CONFIG_PATH="$prefix${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
export LD_LIBRARY_PATH="$prefix${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"

"$app/tools/bindgen.sh"

check_names() {
    lib=$1
    shift
    for name in "$@"; do
        grep -q "fn $name(" "$app/.rnx-cache/generated/$lib/bindgen.rnx" || {
            echo "bindgen output for $lib lost $name" >&2
            exit 1
        }
    done
}

check_names vello_cabi velloAbiVersion velloFill velloRender
check_names parley_cabi parleyAbiVersion parleyClusters parleyMeasure
check_names wgpu_cabi wgpuAdapterCount wgpuAdapterName wgpuClearRgba wgpuInstanceCreate wgpuTriangle wgpuAbiVersion wgpuInstanceDestroy
check_names taffy_cabi taffyAbiVersion taffyLayout
check_names cpu_cabi cpuAbiVersion cpuRender cpuImagePending cpuWaitImages
check_names host_cabi hostAbiVersion hostClose hostEventA hostEventB hostEventC hostGeneration hostHeight hostOpen hostPresent hostPump hostPumpTimeout hostRenderOffscreen hostWait hostWidth
check_names media_cabi mediaAbiVersion mediaAudioOpen mediaVideoOpen mediaCameraOpen mediaPickFile

for wrapper in host media; do
    grep -q "72-platform-support.md" "$app/wrappers/$wrapper/src/lib.rs" || {
        echo "$wrapper lost its Linux-only platform guard" >&2
        exit 1
    }
done

run_wrapper_tests() {
    for w in scene cpu host media; do
        if ! out=$(cd "$app/wrappers/$w" && CARGO_TARGET_DIR="$repo/.rnx-cache/wrapper-target" cargo test --release 2>&1); then
            echo "$out"
            echo "wrapper tests failed for $w" >&2
            exit 1
        fi
        echo "$out" | grep -q "test result: ok" || {
            echo "$out"
            echo "wrapper tests did not pass for $w" >&2
            exit 1
        }
    done
}

run_wrapper_tests

run_probe() {
    w=$1
    shift
    echo "probe: $w"
    if ! out=$(cd "$app/wrappers/$w/probe" && "$rnx" run --backend "$backend"); then
        echo "$out"
        echo "probe $w failed to run" >&2
        exit 1
    fi
    echo "$out"
    for want in "$@"; do
        echo "$out" | grep -q "$want" || {
            echo "probe $w did not print: $want" >&2
            exit 1
        }
    done
}

run_probe wgpu "rgba: 10 20 30 255" "center: 51 204 102 255"
run_probe parley "hello 16:" "clusters:" "rows: 3" "flags: 1" "abi: 6" "grapheme next: 2" "grapheme prev: 0"
run_probe taffy "rc: 0" "node 0 xywh: 0.0 0.0 120.0 78.0"
run_probe cpu "rc: 0" "corner: 20,22,26,255" "heading ink: true" "round center: 200,50,50,255" "moved: 50,200,50,255" "image: 255,0,0,255" "poly center: 50,200,200,255"
run_probe vello "heading ink: true" "corner: 20,22,26,255" "round center: 200,50,50,255" "moved: 50,200,50,255" "image: 255,0,0,255" "poly center: 50,200,200,255"

run_app_tests() {
    (cd "$app/tests" && "$rnx" run --backend "$backend")
}

run_app_tests_llvm() {
    out=$(cd "$app/tests" && "$rnx" run --backend llvm)
    echo "$out" | grep -q "app tests: ok" || {
        echo "app tests failed under the LLVM backend" >&2
        echo "$out" >&2
        exit 1
    }
}

run_surface_socket() {
    out_bin=${TMPDIR:-/tmp}/rnx-surface-probe
    cc -O2 -I "$app/wrappers/host/include" \
        -o "$out_bin" "$app/wrappers/host/probe/surface_socket.c" \
        -L "$prefix" -lhost_cabi
    out=$(LD_LIBRARY_PATH="$prefix${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}" "$out_bin")
    echo "$out"
    echo "$out" | grep -q "surface socket status=0 id=42" || {
        echo "surface socket probe failed" >&2
        exit 1
    }
}

run_multi_window() {
    out_bin=${TMPDIR:-/tmp}/rnx-multi-window
    cc -O2 -I "$app/wrappers/host/include" \
        -o "$out_bin" "$app/wrappers/host/probe/multi_window.c" \
        -L "$prefix" -lhost_cabi
    out=$(LD_LIBRARY_PATH="$prefix${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}" "$out_bin")
    echo "$out"
    echo "$out" | grep -q "count=2 opened=1" || {
        echo "multi-window probe did not open a second toplevel" >&2
        exit 1
    }
    echo "$out" | grep -q "present primary=0 second=0" || {
        echo "multi-window probe did not present both windows" >&2
        exit 1
    }
}

jobs_dir=$(mktemp -d)
job_list=""
launch_job() {
    name=$1
    shift
    "$@" >"$jobs_dir/$name.log" 2>&1 &
    job_list="$job_list $name:$!"
}

launch_job app-tests run_app_tests
launch_job app-tests-llvm run_app_tests_llvm
if command -v cc >/dev/null 2>&1; then
    launch_job surface-socket run_surface_socket
fi
if [ "${RNX_MULTI_PROBE:-}" = "1" ] && [ -n "${WAYLAND_DISPLAY:-}" ] && command -v cc >/dev/null 2>&1; then
    launch_job multi-window run_multi_window
fi

job_fail=0
for entry in $job_list; do
    name=${entry%:*}
    pid=${entry##*:}
    if wait "$pid"; then
        :
    else
        job_fail=1
        echo "--- $name failed" >&2
        cat "$jobs_dir/$name.log" >&2
    fi
done
rm -rf "$jobs_dir"
if [ "$job_fail" -ne 0 ]; then
    exit 1
fi
