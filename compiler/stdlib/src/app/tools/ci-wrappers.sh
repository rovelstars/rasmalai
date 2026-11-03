#!/bin/sh
set -eu
here=$(cd "$(dirname "$0")" && pwd)
app=$(dirname "$here")
repo=$(cd "$app/../../../.." && pwd)
rnx=${RNX_BIN:-$repo/target/debug/rnx}
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
check_names cpu_cabi cpuAbiVersion cpuRender
check_names host_cabi hostAbiVersion hostClose hostEventA hostEventB hostHeight hostOpen hostPresent hostPump hostRenderOffscreen hostWait hostWidth

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
run_probe parley "hello 16:" "clusters:"
run_probe taffy "rc: 0" "node 0 xywh: 0.0 0.0 120.0 78.0"
run_probe cpu "rc: 0" "corner: 20,22,26,255" "heading ink: true" "round center: 200,50,50,255" "moved: 50,200,50,255" "image: 255,0,0,255"
run_probe vello "heading ink: true" "corner: 20,22,26,255" "round center: 200,50,50,255" "moved: 50,200,50,255" "image: 255,0,0,255"

(cd "$app/tests" && "$rnx" run --backend "$backend")
(cd "$repo/examples/app-window" && "$rnx" check)

for profile in dev release; do
    if [ "$profile" = release ]; then
        (cd "$repo/examples/app-window" && "$rnx" build --release)
    else
        (cd "$repo/examples/app-window" && "$rnx" build)
    fi
    bin="$repo/examples/app-window/.rnx-cache/build/$profile/app-window"
    if ldd "$bin" | grep -q "not found"; then
        echo "built app has unresolved libraries" >&2
        ldd "$bin" >&2
        exit 1
    fi
    out=$(cd "$repo/examples/app-window" && RUNIX_SMOKE=1 env -u LD_LIBRARY_PATH "$bin")
    echo "$out" | grep -q "smoke: 0" || {
        echo "built $profile app did not run without LD_LIBRARY_PATH" >&2
        exit 1
    }
done

for b in cranelift llvm; do
    out=$(cd "$repo/examples/app-demo" && "$rnx" run --backend "$b")
    echo "$out"
    echo "$out" | grep -q "heading ink: true"
    echo "$out" | grep -q "status: 0"
done
