#!/usr/bin/env python3
"""Unified multi-language benchmark suite runner.

Runs every benchmark in benches/compute/* and benches/memory/*/ across all
available toolchains (rnx, c, rust, go, node, java, dart), measuring runtime
and peak RSS per (language, mode) configuration (median of 5 runs after 1
warm-up) plus that configuration's build time (median of 3 builds after 1
warm-up), validating cross-language checksums, and emitting
benches/data/benchmarks.json for the website Pareto chart.

Toolchains with a real dev/release switch are measured in both: rnx, c,
rust, dart produce two rows each ("rnx dev", "rnx rel"). go, node, and java
have exactly one honest configuration and produce one row apiece.

Usage:
    python3 benches/harness/runner.py --dry-run   # plan only: toolchains, files, matrix
    python3 benches/harness/runner.py [--benches fib,matmul] [--langs rnx,c]
                                      [--runs 5] [--build-runs 3] [--out benches/data/benchmarks.json]
                                      [--rnx PATH] [--cc clang] [--no-rebuild]

Each benchmark program prints `RESULT <key> <value>` lines. `checksum` must
agree across languages (exact for integers, 1e-6 relative tolerance for
floats); `doclen` (json_stress) must agree exactly.

Timing uses os.wait4() rusage (ru_maxrss, exact, race-free); see
harness/measure.sh for the equivalent standalone wrapper.
"""
import argparse
import datetime
import json
import math
import os
import platform
import shutil
import statistics
import subprocess
import sys
import tempfile
import time

REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
BENCH_DIR = os.path.join(REPO, "benches")
HARNESS_DIR = os.path.join(BENCH_DIR, "harness")
DEFAULT_OUT = os.path.join(BENCH_DIR, "data", "benchmarks.json")


def default_rnx():
    # Prefer an optimized compiler for both release builds and the dev
    # axis; fall back to the debug build with a recorded version string
    # so the provenance stays honest (see versions["rnx_cc"]).
    rel = os.path.join(REPO, "target", "release", "rnx")
    if os.path.isfile(rel) and os.access(rel, os.X_OK):
        return rel
    return os.path.join(REPO, "target", "debug", "rnx")


DEFAULT_RNX = default_rnx()

FLOAT_TOL = 1e-6

BENCHES = [
    {"id": "fib", "dir": "compute/fib", "name": "fib(35)",
     "category": "compute", "float": False,
     "description": "Naive double recursion, call overhead"},
    {"id": "mandelbrot", "dir": "compute/mandelbrot", "name": "mandelbrot 400x400",
     "category": "compute", "float": False,
     "cflags": ["-ffp-contract=off"],
     "rnx_variant": "FastFloat with LIR-synthesized single-rounding FMA; strict prologue",
     "description": "Double float loops, branches, 1000 escape iterations"},
    {"id": "nbody", "dir": "compute/nbody", "name": "nbody 100k steps",
     "category": "compute", "float": True,
     "cflags": ["-ffp-contract=off"],
     "description": "sqrt-heavy float arrays, 5 bodies in Benchmarks-Game units"},
    {"id": "spectral", "dir": "compute/spectral", "name": "spectral N=300",
     "category": "compute", "float": True,
     "cflags": ["-ffp-contract=off"],
     "description": "Array traversals and division, 10 power iterations on AtA"},
    {"id": "matmul", "dir": "compute/matmul", "name": "matmul 256x256",
     "category": "compute", "float": False,
     "description": "Integer arrays, nested loops, bounds checks"},
    {"id": "binary_trees", "dir": "memory/binary_trees", "name": "binary trees depth 16",
     "category": "memory", "float": False,
     "description": "Recursive tree allocation and destruction, struct-of-arrays"},
    {"id": "json_stress", "dir": "memory/json_stress", "name": "json stress 20k objects",
     "category": "memory", "float": False,
     "description": "Payload parsing and dynamic map insertion, 5 parse+serialize rounds"},
]

LANGS = ["rnx", "c", "rust", "go", "node", "java", "dart"]

# A chart point is one (language, mode) pair, so a language whose toolchain
# has a real dev/release switch contributes two points - "rnx dev" and
# "rnx rel" - and the tradeoff (fast build, slow run vs slow build, fast
# run) is visible on the scatter instead of implied by one release row.
#
# `dev` is the configuration a programmer edits with: unoptimized codegen,
# no AOT step. `rel` is the shipping configuration. Toolchains with only
# one honest configuration get one mode; no synthetic flags are invented
# to fill the other slot.
MODES = {
    "rnx": ["dev", "rel"],
    "c": ["dev", "rel"],
    "rust": ["dev", "rel"],
    "go": ["rel"],
    "node": ["rel"],
    "java": ["rel"],
    "dart": ["dev", "rel"],
}

# Exact build step per (language, mode), recorded on every result row. For
# C the per-bench float flags are appended at build time.
BUILD = {
    ("rnx", "dev"): "rnx build (LLVM AOT dev: unoptimized object + linked runnable binary)",
    ("rnx", "rel"): "rnx build --release (LLVM AOT: program default<O3> host-cpu; runtime archive -O3 generic)",
    ("c", "dev"): "clang -O0",
    ("c", "rel"): "clang -O3 -march=native",
    ("rust", "dev"): "rustc (unoptimized, no LTO)",
    ("rust", "rel"): "rustc -C opt-level=3 -C codegen-units=1",
    ("go", "rel"): "go build (only mode: Go always compiles optimized, no dev build exists)",
    ("node", "rel"): "node --check (interpreted, V8 JIT: no compile step exists)",
    ("java", "rel"): "javac (only mode: javac has no -O; HotSpot tiers its own JIT)",
    ("dart", "dev"): "dart compile kernel (JIT snapshot, no AOT; run with `dart run`)",
    ("dart", "rel"): "dart compile exe (AOT native)",
}

# What the timed run actually executes, for the modes that do not run a
# linked artifact.
RUN = {
    ("node", "rel"): "node <file>",
    ("dart", "dev"): "dart run <snapshot>",
    ("java", "rel"): "java -cp <classes> <Class>",
}

SRC = {
    "fib": {"rnx": "main.rnx", "c": "fib.c", "rust": "fib.rs", "go": "fib.go",
            "node": "fib.js", "java": "Fib.java", "dart": "fib.dart"},
    "mandelbrot": {"rnx": "main.rnx", "c": "mandelbrot.c", "rust": "mandelbrot.rs",
                   "go": "mandelbrot.go", "node": "mandelbrot.js",
                   "java": "Mandelbrot.java", "dart": "mandelbrot.dart"},
    "nbody": {"rnx": "main.rnx", "c": "nbody.c", "rust": "nbody.rs", "go": "nbody.go",
              "node": "nbody.js", "java": "Nbody.java", "dart": "nbody.dart"},
    "spectral": {"rnx": "main.rnx", "c": "spectral.c", "rust": "spectral.rs",
                 "go": "spectral.go", "node": "spectral.js",
                 "java": "Spectral.java", "dart": "spectral.dart"},
    "matmul": {"rnx": "main.rnx", "c": "matmul.c", "rust": "matmul.rs", "go": "matmul.go",
               "node": "matmul.js", "java": "Matmul.java", "dart": "matmul.dart"},
    "binary_trees": {"rnx": "main.rnx", "c": "binary_trees.c", "rust": "binary_trees.rs",
                     "go": "binary_trees.go", "node": "binary_trees.js",
                     "java": "BinaryTrees.java", "dart": "binary_trees.dart"},
    "json_stress": {"rnx": "main.rnx", "c": "json_stress.c", "rust": "json_stress.rs",
                    "go": "json_stress.go", "node": "json_stress.js",
                    "java": "JsonStress.java", "dart": "json_stress.dart"},
}


def run_timed(argv, cwd):
    """Run argv, return (rc, stdout, wall_ms, peak_kb) via the native helper.

    Delegates to harness/measure.sh, which execs the compiled
    harness/measure helper: a ~15 KB parent that fork/execvps the target
    and reports the child's own wait4 rusage as JSON. The parent never
    maps a heap or interpreter runtime, so no copy-on-write pages leak
    into the child's ru_maxrss. Single code path for all languages.
    """
    fd, tmppath = tempfile.mkstemp(prefix="bench-out-")
    os.close(fd)
    try:
        proc = subprocess.run(
            [os.path.join(HARNESS_DIR, "measure.sh"), tmppath] + list(argv),
            stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
            cwd=cwd, text=True,
        )
        rc, wall_ms, peak = 127, 0.0, 0
        try:
            m = json.loads(proc.stdout.strip().splitlines()[-1])
            rc = int(m["exit_code"])
            wall_ms = float(m["elapsed_ns"]) / 1e6
            peak = int(m["max_rss_bytes"]) // 1024
        except (ValueError, KeyError, IndexError, TypeError):
            for token in proc.stdout.split():
                if token.startswith("RC="):
                    rc = int(token[3:])
                elif token.startswith("WALL_MS="):
                    wall_ms = float(token[8:])
                elif token.startswith("PEAK_KB="):
                    peak = int(token[8:])
        with open(tmppath) as f:
            out = f.read()
    finally:
        try:
            os.remove(tmppath)
        except OSError:
            pass
    return rc, out, wall_ms, peak


def run_simple(argv, cwd):
    """Run argv, return (rc, stdout, wall_ms); stdout+stderr captured as text."""
    t0 = time.perf_counter()
    proc = subprocess.run(argv, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                          cwd=cwd, text=True)
    return proc.returncode, proc.stdout, (time.perf_counter() - t0) * 1000.0


def parse_results(stdout):
    vals = {}
    for line in stdout.splitlines():
        line = line.strip()
        if line.startswith("RESULT"):
            parts = line.split(None, 2)
            if len(parts) == 3:
                vals.setdefault(parts[1], []).append(parts[2])
    return vals


class Toolchains:
    def __init__(self, args):
        self.rnx = args.rnx
        self.cc = args.cc
        self.tmp = tempfile.mkdtemp(prefix="rnx-bench-")
        self.status = {}
        self.detect()

    def detect(self):
        self.status["rnx"] = self._ok_cmd([self.rnx, "--version"], "rnx")
        self.status["c"] = self._ok_cmd([self.cc, "--version"], "c compiler")
        self.status["rust"] = self._ok_cmd(["rustc", "--version"], "rustc")
        self.status["go"] = self._ok_cmd(["go", "version"], "go")
        self.status["node"] = self._ok_cmd(["node", "--version"], "node")
        self.status["java"] = self._java()
        self.status["dart"] = self._ok_cmd(["dart", "--version"], "dart")

    def _ok_file(self, path, what):
        if path and os.path.isfile(path) and os.access(path, os.X_OK):
            return (True, f"{what}: {path}")
        return (False, f"{what} not found: {path}")

    def _ok_cmd(self, argv, what):
        if shutil.which(argv[0]) is None:
            return (False, f"{what}: {argv[0]} not on PATH")
        try:
            rc, out, _ = run_simple(argv, REPO)
            if rc == 0:
                return (True, f"{what}: {out.strip().splitlines()[0][:80]}")
        except OSError:
            pass
        return (False, f"{what}: {argv[0]} failed to run")

    def _java(self):
        if shutil.which("javac") is not None:
            rc, out, _ = run_simple(["javac", "--version"], REPO)
            if rc == 0:
                return (True, f"javac: {out.strip()[:80]}")
        if shutil.which("java") is not None:
            hello = os.path.join(self.tmp, "Probe.java")
            with open(hello, "w") as f:
                f.write('public class Probe { public static void main(String[] a) { System.out.println("hi"); } }\n')
            rc, out, _ = run_simple(["java", hello], REPO)
            if rc == 0 and "hi" in out:
                return (True, "java single-file mode (no javac)")
            return (False, "java runtime present but broken (single-file probe failed)")
        return (False, "no java/javac on PATH")

    def available(self, lang):
        return self.status.get(lang, (False, "unknown"))[0]

    def version_line(self, lang):
        ok, msg = self.status.get(lang, (False, "unknown"))
        return msg if ok else "unavailable"


def artifact_path(workdir, lang, mode):
    name = f"bench_{lang}_{mode}"
    if lang == "dart" and mode == "dev":
        name += ".dill"
    return os.path.join(workdir, name)


def build_argv(tc, lang, mode, bench, src_path, workdir):
    """The timed build step for this (language, mode), and whether it
    produces the runnable artifact.

    Node has no compile step, so its timed step is a parse check flagged
    artifact=False: the chart must never imply Node compiles anything.
    """
    if lang == "node":
        return ["node", "--check", src_path], False
    if lang == "rnx":
        flags = ["--release"] if mode == "rel" else []
        return [tc.rnx, "build"] + flags + [src_path], True
    if lang == "c":
        # Strict IEEE-754 everywhere: -ffp-contract=off on every float
        # bench (mandelbrot, nbody, spectral). Without it clang fuses
        # a*b+c into FMA under -march=native while rnx (Float is
        # spec-bound strict), rustc, Go, HotSpot, V8 and Dart all
        # evaluate strictly - and the 1e-6 checksum tolerance hides it.
        opt = ["-O3", "-march=native"] if mode == "rel" else ["-O0"]
        return [tc.cc] + opt + bench.get("cflags", []) + \
            [src_path, "-o", artifact_path(workdir, lang, mode), "-lm"], True
    if lang == "rust":
        opt = ["-C", "opt-level=3", "-C", "codegen-units=1"] if mode == "rel" else []
        return ["rustc", "--edition", "2021"] + opt + \
            [src_path, "-o", artifact_path(workdir, lang, mode)], True
    if lang == "go":
        return ["go", "build", "-o", artifact_path(workdir, lang, mode), src_path], True
    if lang == "java":
        if shutil.which("javac") is not None:
            return ["javac", "-d", workdir, src_path], True
        return None, False
    if lang == "dart":
        sub = "exe" if mode == "rel" else "kernel"
        return ["dart", "compile", sub, src_path, "-o", artifact_path(workdir, lang, mode)], True
    return None, False


def run_argv(tc, lang, mode, src_path, workdir):
    """argv that runs what build_argv produced for this mode."""
    if lang == "dart" and mode == "dev":
        # A kernel snapshot is not an executable; the VM loads it.
        return ["dart", "run", artifact_path(workdir, lang, mode)]
    if lang == "node":
        return ["node", src_path]
    if lang == "java":
        cls = os.path.splitext(os.path.basename(src_path))[0]
        if shutil.which("javac") is not None:
            return ["java", "-cp", workdir, cls]
        return ["java", src_path]
    return [artifact_path(workdir, lang, mode)]


def time_build(argv, cwd, runs):
    """Median ms for a build step, after one untimed warm-up so the
    reported number is a steady-state build, not a cold page cache.
    Returns (ms, None) or (None, error text)."""
    rc, msg, _ = run_simple(argv, cwd)
    if rc != 0:
        return None, msg[-1500:]
    walls = []
    for _ in range(runs):
        _, _, ms = run_simple(argv, cwd)
        walls.append(ms)
    return statistics.median(walls), None


def check_agrees(ref, got, is_float):
    if ref is None:
        return True
    if is_float:
        try:
            a, b = float(ref), float(got)
        except ValueError:
            return ref == got
        if math.isnan(a) and math.isnan(b):
            return True
        return math.isclose(a, b, rel_tol=FLOAT_TOL, abs_tol=FLOAT_TOL)
    return ref == got


def main():
    ap = argparse.ArgumentParser(description="Unified multi-language benchmark runner")
    ap.add_argument("--dry-run", action="store_true", help="print plan and exit")
    ap.add_argument("--benches", default=",".join(b["id"] for b in BENCHES))
    ap.add_argument("--langs", default=",".join(LANGS))
    ap.add_argument("--runs", type=int, default=5, help="timed runs per config (plus 1 warm-up)")
    ap.add_argument("--build-runs", type=int, default=3, help="timed builds per config (plus 1 warm-up)")
    ap.add_argument("--out", default=DEFAULT_OUT)
    ap.add_argument("--rnx", default=DEFAULT_RNX)
    ap.add_argument("--cc", default="clang" if shutil.which("clang") else "gcc")
    ap.add_argument("--no-rebuild", action="store_true",
                    help="skip release builds (binaries already in workdirs)")
    args = ap.parse_args()

    benches = [b for b in BENCHES if b["id"] in args.benches.split(",")]
    langs = [l for l in args.langs.split(",") if l in LANGS]
    tc = Toolchains(args)

    print("# Benchmark plan")
    print(f"# repo: {REPO}")
    for lang in LANGS:
        ok, msg = tc.status[lang]
        print(f"#   [{('OK ' if ok else 'SKIP')}] {lang}: {msg}")
    missing_files = []
    for b in benches:
        for lang in langs:
            if not tc.available(lang):
                continue
            src = os.path.join(BENCH_DIR, b["dir"], SRC[b["id"]][lang])
            if not os.path.isfile(src):
                missing_files.append(src)
    if missing_files:
        print("# MISSING sources:")
        for f in missing_files:
            print(f"#   {f}")
        return 2
    print(f"# benches: {', '.join(b['id'] for b in benches)}")
    print(f"# langs: {', '.join(langs)} (runnable: {', '.join(l for l in langs if tc.available(l))})")
    if args.dry_run:
        print("# dry-run: plan valid, no builds executed.")
        return 0

    try:
        with open("/proc/cpuinfo") as f:
            cpu = next((l.split(":", 1)[1].strip() for l in f if "model name" in l), "unknown")
    except OSError:
        cpu = platform.processor() or "unknown"

    failures = []
    data = {
        "system": {
            "cpu": cpu,
            "os": platform.system(),
            "date": datetime.date.today().isoformat(),
        },
        "versions": {lang: tc.version_line(lang) for lang in langs},
        "rnx_compiler": "release" if "/release/" in args.rnx else "debug",
        "benchmarks": {},
    }

    for b in benches:
        bid = b["id"]
        print(f"\n## {bid}", flush=True)
        results = []
        ref_sum = None
        ref_sum_rnx = None
        ref_doclen = None
        for lang in langs:
            if not tc.available(lang):
                print(f"  {lang}: skipped ({tc.status[lang][1]})")
                continue
            src = os.path.join(BENCH_DIR, b["dir"], SRC[bid][lang])
            workdir = os.path.join(tc.tmp, bid, lang)
            os.makedirs(workdir, exist_ok=True)
            for mode in MODES[lang]:
                tag = f"{lang} {mode}"
                cfg, artifact = build_argv(tc, lang, mode, b, src, workdir)
                build_ms = 0.0
                if cfg is not None:
                    build_ms, err = time_build(cfg, REPO, args.build_runs)
                    if build_ms is None:
                        failures.append(f"{bid}/{tag}: {err}")
                        print(f"  {tag}: BUILD FAIL")
                        continue
                    if lang == "rnx":
                        # rnx chooses its own output path under .rnx-cache and
                        # prints `artifact: <path>`; stage a copy at the path
                        # run_argv expects (the rebuild is a fresh-cache hit).
                        rc2, out2, _ = run_simple(cfg, REPO)
                        real = None
                        for line in out2.splitlines():
                            line = line.strip()
                            if line.startswith("artifact: "):
                                real = line[len("artifact: "):].removesuffix(" (fresh)").strip()
                        if rc2 != 0 or not real or not os.path.isfile(real):
                            failures.append(f"{bid}/{tag}: artifact resolve failed")
                            print(f"  {tag}: BUILD FAIL")
                            continue
                        shutil.copy(real, artifact_path(workdir, lang, mode))
                argv = run_argv(tc, lang, mode, src, workdir)
                walls, peaks = [], []
                out = ""
                ok = True
                for run in range(args.runs + 1):
                    rc, out, wall, peak = run_timed(argv, REPO)
                    if rc != 0:
                        failures.append(f"{bid}/{tag}: exit {rc}\n{out[-1000:]}")
                        print(f"  {tag}: RUN FAIL (exit {rc})")
                        ok = False
                        break
                    if run > 0:
                        walls.append(wall)
                        peaks.append(peak)
                if not ok:
                    continue
                vals = parse_results(out)
                sums = vals.get("checksum", [])
                if not sums:
                    failures.append(f"{bid}/{tag}: no RESULT checksum in output")
                    print(f"  {tag}: NO CHECKSUM")
                    continue
                if b.get("rnx_variant"):
                    ref = ref_sum_rnx if lang == "rnx" else ref_sum
                    if not check_agrees(ref, sums[0], b["float"]):
                        failures.append(f"{bid}/{tag}: checksum {sums[0]} != ref {ref}")
                        print(f"  {tag}: CHECKSUM MISMATCH ({sums[0]} vs {ref})")
                        continue
                    if lang == "rnx" and ref_sum_rnx is None:
                        ref_sum_rnx = sums[0]
                    if lang != "rnx" and ref_sum is None:
                        ref_sum = sums[0]
                else:
                    if not check_agrees(ref_sum, sums[0], b["float"]):
                        failures.append(f"{bid}/{tag}: checksum {sums[0]} != ref {ref_sum}")
                        print(f"  {tag}: CHECKSUM MISMATCH ({sums[0]} vs {ref_sum})")
                        continue
                    if ref_sum is None:
                        ref_sum = sums[0]
                if "doclen" in vals:
                    if ref_doclen is None:
                        ref_doclen = vals["doclen"][0]
                    elif vals["doclen"][0] != ref_doclen:
                        failures.append(f"{bid}/{tag}: doclen {vals['doclen'][0]} != {ref_doclen}")
                        print(f"  {tag}: DOCLEN MISMATCH")
                        continue
                runtime = statistics.median(walls)
                peak_mb = statistics.median(peaks) / 1024.0
                print(f"  {tag}: {runtime:.1f} ms / {peak_mb:.1f} MB / build {build_ms:.1f} ms  [{sums[0]}]")
                desc = BUILD[(lang, mode)]
                if lang == "c" and b.get("cflags"):
                    desc = desc + " " + " ".join(b["cflags"])
                results.append({
                    "lang": lang,
                    "mode": mode,
                    "label": tag,
                    "runtime_ms": round(runtime, 1),
                    "peak_rss_mb": round(peak_mb, 1),
                    "build_ms": round(build_ms, 1),
                    "build": desc,
                    "run": RUN.get((lang, mode), "linked artifact from the build step"),
                    "artifact": artifact,
                })
        data["benchmarks"][bid] = {
            "name": f"{b['name']}",
            "category": b["category"],
            "description": b["description"],
            "results": results,
        }
        if b.get("rnx_variant"):
            data["benchmarks"][bid]["rnx_variant"] = b["rnx_variant"]
            data["benchmarks"][bid]["checksum_ref"] = ref_sum
            data["benchmarks"][bid]["checksum_ref_rnx"] = ref_sum_rnx

    os.makedirs(os.path.dirname(args.out), exist_ok=True)
    with open(args.out, "w") as f:
        json.dump(data, f, indent=2)
        f.write("\n")
    print(f"\nwrote {args.out}")

    ok = True
    for bid, bench in data["benchmarks"].items():
        seen = set()
        for r in bench["results"]:
            tag = f"{bid}/{r['label']}"
            for k in ("runtime_ms", "peak_rss_mb", "build_ms"):
                v = r[k]
                if not isinstance(v, (int, float)) or not math.isfinite(v) or v < 0:
                    failures.append(f"{tag}: invalid {k}={v!r}")
                    ok = False
            if r["mode"] not in ("dev", "rel"):
                failures.append(f"{tag}: mode must be dev or rel")
                ok = False
            if r["label"] in seen:
                failures.append(f"{tag}: duplicate label")
                ok = False
            seen.add(r["label"])
            for k in ("build", "run"):
                if not isinstance(r.get(k), str) or not r[k]:
                    failures.append(f"{tag}: missing {k} provenance")
                    ok = False
            if not isinstance(r.get("artifact"), bool):
                failures.append(f"{tag}: missing artifact flag")
                ok = False
            if r["build_ms"] == 0 and r["artifact"]:
                failures.append(f"{tag}: build_ms 0 for a config that builds an artifact")
                ok = False
    if failures:
        print("\n## FAILURES")
        for f in failures:
            print(f"- {f}")
        return 1
    if not ok:
        return 1
    print("checksums agree across languages; all numeric fields valid.")
    return 0


if __name__ == "__main__":
    sys.exit(main())