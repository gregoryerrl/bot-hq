"""Leakage-free test verification for the test-feedback loop.

Runs an instance's EXISTING tests (PASS_TO_PASS) against the agent's current
diff inside the prebuilt SWE-bench container, applying ONLY the diff — never the
hidden test_patch. Returns the existing tests the diff broke (regressions), which
the rollout feeds back to the duo to revise.

Image caching: the first pull of a repo's eval image is slow (~40 min emulated),
but Docker caches it, so subsequent instances/rounds of that repo reuse it.
"""
from __future__ import annotations

import ast
import os
import subprocess
import tempfile


def _make_spec(inst):
    try:
        from swebench.harness.test_spec.test_spec import make_test_spec
    except ImportError:
        from swebench.harness.test_spec import make_test_spec
    try:
        return make_test_spec(inst, namespace="swebench")
    except TypeError:
        return make_test_spec(inst)


def _existing_test_files(inst) -> list[str]:
    p = inst.get("PASS_TO_PASS")
    tests = ast.literal_eval(p) if isinstance(p, str) else (p or [])
    return sorted({t.split("::")[0] for t in tests})


# Inside the container: activate testbed, apply ONLY the diff, reinstall, run the
# existing test files. No test_patch. Any FAILED here is a regression the diff caused.
_SCRIPT = r"""set +e
source /opt/miniconda3/bin/activate && conda activate testbed
cd /testbed
git checkout -- . 2>/dev/null || true
if ! git apply /tmp/model.patch 2>/dev/null; then
  git apply --3way /tmp/model.patch 2>/dev/null || echo "__PATCH_APPLY_FAILED__"
fi
python -m pip install . -q 2>&1 | tail -1
python -m pytest -rA --tb=line --no-header -q __TESTS__ 2>&1 | tail -160
"""

_pulled: set[str] = set()


def _ensure_image(img: str) -> bool:
    if img in _pulled:
        return True
    if subprocess.run(["docker", "image", "inspect", img], capture_output=True).returncode == 0:
        _pulled.add(img)
        return True
    r = subprocess.run(["docker", "pull", "--platform", "linux/amd64", img], capture_output=True, text=True)
    if r.returncode == 0:
        _pulled.add(img)
        return True
    return False


def verify_existing_tests(inst: dict, diff_text: str):
    """Run existing tests against `diff_text` (model_patch only) in the prebuilt
    container. Returns (failing_test_ids, summary, ok, details). `details` holds
    the top failing tests WITH their error lines (`FAILED test - ErrorType: msg`)
    so the duo can debug, not just see names. `ok` is False on an infra problem
    (image pull / apply) so the caller can skip feedback rather than misread it
    as 'clean'."""
    if not diff_text.strip():
        return [], "empty patch — nothing to verify", True, ""
    img = _make_spec(inst).instance_image_key
    if not _ensure_image(img):
        return [], f"image pull failed: {img}", False, ""
    files = _existing_test_files(inst)
    test_arg = " ".join(files[:5]) or "."
    with tempfile.NamedTemporaryFile("w", suffix=".patch", delete=False) as f:
        f.write(diff_text)
        pf = f.name
    script = _SCRIPT.replace("__TESTS__", test_arg)
    try:
        p = subprocess.run(
            ["docker", "run", "--rm", "--platform", "linux/amd64",
             "-v", f"{pf}:/tmp/model.patch", "--entrypoint", "bash", img, "-c", script],
            capture_output=True, text=True, timeout=2400,
        )
    finally:
        os.unlink(pf)
    out = p.stdout
    if "__PATCH_APPLY_FAILED__" in out:
        return [], "patch did not apply in container", False, ""
    failed_lines = [ln for ln in out.splitlines() if ln.startswith("FAILED ")]
    failing = [ln.split()[1] for ln in failed_lines if len(ln.split()) > 1]
    summary = next((ln.strip() for ln in reversed(out.splitlines())
                    if "passed" in ln or "failed" in ln or "error" in ln), "no pytest summary")
    # the `FAILED <test> - <ErrorType: msg>` lines (from -rA) are the actionable
    # signal: each failing test plus its error. Cap to keep the message bounded.
    details = "\n".join(failed_lines[:12])
    return failing, summary, True, details
