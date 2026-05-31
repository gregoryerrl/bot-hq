#!/usr/bin/env python3
"""Leakage-free verify: run an instance's EXISTING tests against the duo's
model_patch inside the prebuilt SWE-bench container — applying ONLY the
model_patch, never the hidden test_patch. This is the core of the planned
test-feedback loop; this script proves the mechanism on a known-wrong instance.

Usage: .venv/bin/python poc_verify.py psf__requests-1724
"""
from __future__ import annotations

import ast
import json
import os
import subprocess
import sys
import tempfile

import dataset


def make_spec(inst):
    try:
        from swebench.harness.test_spec.test_spec import make_test_spec
    except ImportError:
        from swebench.harness.test_spec import make_test_spec
    try:
        return make_test_spec(inst, namespace="swebench")
    except TypeError:
        return make_test_spec(inst)


def model_patch(iid: str):
    for fn in ("runs/combined/predictions39.jsonl", "runs/scaleup/predictions.jsonl",
               "runs/scaleup2/predictions.jsonl", "runs/smoke/predictions.jsonl"):
        if not os.path.exists(fn):
            continue
        for line in open(fn):
            d = json.loads(line)
            if d["instance_id"] == iid:
                return d["model_patch"]
    return None


def existing_tests(inst) -> list[str]:
    p = inst.get("PASS_TO_PASS")
    return ast.literal_eval(p) if isinstance(p, str) else (p or [])


# Inside the container: activate the testbed env, apply ONLY the model_patch,
# reinstall, run the EXISTING tests. No test_patch is ever applied.
_VERIFY = r"""set +e
source /opt/miniconda3/bin/activate && conda activate testbed
cd /testbed
git checkout -- . 2>/dev/null || true
echo ">>> applying model_patch (leakage-free: NO test_patch)"
if git apply -v /tmp/model.patch 2>&1 | tail -2; then :; else
  echo "(direct apply failed; trying --3way)"; git apply --3way /tmp/model.patch 2>&1 | tail -2
fi
python -m pip install . -q 2>&1 | tail -1
echo ">>> running EXISTING tests: __TESTS__"
python -m pytest -rA __TESTS__ 2>&1 | tail -45
"""


def main():
    iid = sys.argv[1] if len(sys.argv) > 1 else "psf__requests-1724"
    insts, _ = dataset.load_by_ids([iid])
    if not insts:
        print(f"instance {iid} not found")
        return
    inst = insts[0]
    patch = model_patch(iid)
    if not patch or not patch.strip():
        print(f"no (non-empty) model_patch for {iid}")
        return
    spec = make_spec(inst)
    img = spec.instance_image_key
    tests = existing_tests(inst)
    testfiles = sorted({t.split("::")[0] for t in tests})
    test_arg = " ".join(testfiles[:3]) or "."
    print(f"instance = {iid}")
    print(f"image    = {img}")
    print(f"existing tests = {len(tests)} across {testfiles[:5]}")
    print("pulling image (slow under emulation)…", flush=True)
    r = subprocess.run(["docker", "pull", "--platform", "linux/amd64", img], capture_output=True, text=True)
    if r.returncode != 0:
        print("PULL FAILED:\n", r.stderr[-600:])
        return
    with tempfile.NamedTemporaryFile("w", suffix=".patch", delete=False) as f:
        f.write(patch)
        patchfile = f.name
    script = _VERIFY.replace("__TESTS__", test_arg)
    print(f"running leakage-free verify against: {test_arg}", flush=True)
    p = subprocess.run(
        ["docker", "run", "--rm", "--platform", "linux/amd64",
         "-v", f"{patchfile}:/tmp/model.patch", "--entrypoint", "bash", img, "-c", script],
        capture_output=True, text=True, timeout=2400,
    )
    os.unlink(patchfile)
    print("=== container output (tail) ===")
    print(p.stdout[-3500:])
    if p.stderr.strip():
        print("--- stderr (tail) ---")
        print(p.stderr[-800:])


if __name__ == "__main__":
    main()
