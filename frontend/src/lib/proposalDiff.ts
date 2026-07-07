// Minimal LCS line diff for proposal review: current CL file vs proposed
// body. CL files are small one-liner notes, so the O(n·m) table is fine under
// the guard below; the fallback degrades to remove-all/add-all rather than
// blowing up on a pathological input.

export interface ProposalDiffLine {
  kind: "add" | "remove" | "context";
  text: string;
}

const MAX_CELLS = 4_000_000; // ~2000×2000 lines — far beyond any sane CL file

export function diffLines(current: string, proposed: string): ProposalDiffLine[] {
  const a = current.split("\n");
  const b = proposed.split("\n");
  if (a.length * b.length > MAX_CELLS) {
    return [
      ...a.map((text) => ({ kind: "remove" as const, text })),
      ...b.map((text) => ({ kind: "add" as const, text })),
    ];
  }
  const n = a.length;
  const m = b.length;
  // dp[i][j] = LCS length of a[i..] vs b[j..], flattened row-major.
  const dp = new Uint32Array((n + 1) * (m + 1));
  const idx = (i: number, j: number) => i * (m + 1) + j;
  for (let i = n - 1; i >= 0; i--) {
    for (let j = m - 1; j >= 0; j--) {
      dp[idx(i, j)] =
        a[i] === b[j]
          ? dp[idx(i + 1, j + 1)] + 1
          : Math.max(dp[idx(i + 1, j)], dp[idx(i, j + 1)]);
    }
  }
  const out: ProposalDiffLine[] = [];
  let i = 0;
  let j = 0;
  while (i < n && j < m) {
    if (a[i] === b[j]) {
      out.push({ kind: "context", text: a[i] });
      i++;
      j++;
    } else if (dp[idx(i + 1, j)] >= dp[idx(i, j + 1)]) {
      out.push({ kind: "remove", text: a[i] });
      i++;
    } else {
      out.push({ kind: "add", text: b[j] });
      j++;
    }
  }
  for (; i < n; i++) out.push({ kind: "remove", text: a[i] });
  for (; j < m; j++) out.push({ kind: "add", text: b[j] });
  return out;
}
