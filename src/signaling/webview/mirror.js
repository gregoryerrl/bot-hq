// The host mirrors its lease generation into the window after every acquire,
// renew, release or takeover. Monotonic: an update that reaches the window
// late never rolls the mirror back. Returns what the window now holds.
((args) => {
  const current = typeof window.__bhq_lease_gen === "number" ? window.__bhq_lease_gen : 0;
  if (args.gen > current) window.__bhq_lease_gen = args.gen;
  return { outcome: "done", ok: true, matched: true, gen: window.__bhq_lease_gen, requested: args.gen, applied: args.gen > current };
})
