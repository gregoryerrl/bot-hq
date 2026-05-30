"""Format a SWE-bench instance into the message sent to the bot-hq duo."""
from __future__ import annotations

COMPLETION_SENTINEL = "SWEBENCH_DONE"

_TEMPLATE = """\
You are resolving a real GitHub issue in the `{repo}` repository. The repository \
is already checked out at the relevant base commit in your working directory \
({repo_path}); all the source you need is on disk.

<issue>
{problem_statement}
</issue>

Instructions:
- Make the MINIMAL source-code change that resolves the issue described above.
- Edit files directly in the working repository. Do NOT create or modify test \
files — the evaluator applies its own tests.
- Do NOT commit, do NOT push, do NOT open a pull request. Leave the fix as \
uncommitted working-tree changes; the harness extracts your patch via `git diff`.
- Rain (EYES) should review the change for correctness before you finish.
- There is no human available to answer questions. When the fix is complete and \
reviewed, Brian (HANDS) must call `mark_awaiting_user` with exactly the text \
"{sentinel}" to signal completion. Do not wait on any other input.
"""


def format_prompt(instance: dict, repo_path: str) -> str:
    return _TEMPLATE.format(
        repo=instance.get("repo", "?"),
        repo_path=repo_path,
        problem_statement=(instance.get("problem_statement") or "").strip(),
        sentinel=COMPLETION_SENTINEL,
    )
