# Sentinel Content-Shape Corpus (B4) — Phase J Tier-1 T1.3 Impl Input

**Author:** Rain (BRAIN/EYES, read-only investigation per DISC v2 + Phase J Q1 markdown-design-doc HANDS scope)
**Date:** 2026-04-29
**Driver:** Phase J T1.3 (B4) — sentinel content-shape discrimination to eliminate user-prose false-positives while preserving coverage of pasted Go-runtime-fatal output
**Source-substrate:** `internal/gemma/sentinel.go:42-65` (preFilterPatterns + alwaysFlagPatterns) + msgs 4977/4978 (today's FP exhibit)

---

## 1. Goal

Provide T1.3 impl with:
- **TP corpus** — Go-runtime-fatal output shapes that MUST fire the sentinel (paste-from-terminal class)
- **FP corpus** — prose containing trigger-words that MUST NOT fire (Q&A, design-discussion, error-reference-without-paste class)
- **Edge cases** — ambiguous shapes; impl decides per-case
- **Recommended discrimination patterns** — regex shape-anchors that achieve TP/FP separation

T1.3 impl is Brian-HANDS (sentinel.go is `internal/gemma/`, outside Rain markdown/test-file scope per Q1). This doc is Rain investigation deliverable feeding T1.3 design.

---

## 2. Current sentinel state (pre-T1.3)

### 2.1 preFilterPatterns (volume gate, sentinel.go:42-53)

Match → message enters classifier. Non-match → silent drop.

| # | Pattern source                                        | Shape-anchored? | Notes                              |
| - | ----------------------------------------------------- | --------------- | ---------------------------------- |
| 1 | `(?i)\bpanic(:\|\()`                                  | YES             | `panic:` or `panic(` — Go canonical |
| 2 | `(?i)\bfatal\b`                                       | **NO** ← FP source | bare word — exhibit msg 4977   |
| 3 | `(?i)\bdeadlock!`                                     | YES             | trailing `!` is Go runtime emit    |
| 4 | `(?i)rate[\s\-]?limit`                                | NO              | bare phrase                        |
| 5 | `(?i)\bOOM\b\|out[\s\-]of[\s\-]memory`                | NO              | bare phrase                        |
| 6 | `(?i)process\s+exit(ed)?`                             | NO              | bare phrase                        |
| 7 | `(?i)schema\s+constraint\s+(violation\|failed\|error)`| Partial         | "schema constraint" prefix         |
| 8 | `(?i)stack\s+overflow`                                | NO              | bare phrase                        |
| 9 | `(?i)segmentation\s+fault\|SIGSEGV`                   | Partial (SIGSEGV is shape-specific) | mixed |
| 10| `queueFailPattern` (`\[queue\] Message <id> to <agent> failed after <N> attempts`) | YES (in dry-run gate) | bot-hq emit-shape |

### 2.2 alwaysFlagPatterns (strict subset, sentinel.go:58-65)

Match → MsgFlag elevation (rate-cap + hysteresis still apply via shouldFlag).

| # | Pattern                                               | Notes                              |
| - | ----------------------------------------------------- | ---------------------------------- |
| 1 | `(?i)\bpanic(:\|\()`                                  | shape-anchored                     |
| 2 | `(?i)\bdeadlock!`                                     | shape-anchored                     |
| 3 | `(?i)rate[\s\-]?limit`                                | LOOSE                              |
| 4 | `(?i)process\s+exit(ed)?`                             | LOOSE                              |
| 5 | `(?i)schema\s+constraint\s+(violation\|failed\|error)`| partial                            |
| 6 | `(?i)segmentation\s+fault\|SIGSEGV`                   | partial                            |

**Note:** `\bfatal\b` is in preFilterPatterns but NOT alwaysFlagPatterns directly. The msg 4977 FLAG fire suggests a downstream alwaysFlag promotion path (worth verifying in T1.3 trace) — could be that any preFilter match elevates to MsgFlag through a separate code path, OR that the user-context shows fatal as alwaysFlag-class. Either way, the T1.3 fix targets the prose-FP class regardless of which list fires.

---

## 3. TP corpus — must fire

These are the shapes the sentinel MUST continue to catch. Source: paste-from-terminal class — user pastes actual Go-runtime output to share/debug.

### TP-1: classic Go panic stack trace

```
panic: runtime error: invalid memory address or nil pointer dereference
[signal SIGSEGV: segmentation violation code=0x1 addr=0x0 pc=0x4f8a23]

goroutine 1 [running]:
main.process(0x0)
	/Users/gregoryerrl/Projects/bot-hq/internal/main.go:42 +0x23
main.main()
	/Users/gregoryerrl/Projects/bot-hq/internal/main.go:31 +0x11
exit status 2
```

**Anchors:** `panic:` (line-leading), `goroutine N [running]:`, `signal SIGSEGV:`, `exit status N`.

### TP-2: fatal error (Go runtime, unrecoverable)

```
fatal error: concurrent map writes

goroutine 7 [running]:
runtime.throw({0x102adc4f1?, 0x4?})
	/opt/homebrew/Cellar/go/1.22.4/libexec/src/runtime/panic.go:1023 +0x40
runtime.mapassign_faststr(0x102b8e7c0, 0xc000098000, {0x102ac6abf, 0x6})
	/opt/homebrew/Cellar/go/1.22.4/libexec/src/runtime/map_faststr.go:295 +0x39c
```

**Anchors:** `fatal error:` (line-leading), `runtime.throw`, `goroutine N [running]:`.
**Critical:** `fatal error:` (with colon, line-leading) is the Go runtime canonical format — distinct from prose "fatal" usage.

### TP-3: deadlock detection

```
fatal error: all goroutines are asleep - deadlock!

goroutine 1 [chan receive]:
main.main()
	/tmp/deadlock.go:9 +0x4d
exit status 2
```

**Anchors:** `fatal error:` + `deadlock!` + `goroutine N [chan receive]:`.

### TP-4: stack overflow

```
runtime: goroutine stack exceeds 1000000000-byte limit
runtime: sp=0xc020100378 stack=[0xc020100000, 0xc040100000]
fatal error: stack overflow

goroutine 1 [running]:
runtime.throw({0x4eda9c?, 0x16?})
```

**Anchors:** `runtime: goroutine stack exceeds`, `fatal error: stack overflow`, `runtime.throw`.

### TP-5: out of memory

```
runtime: out of memory: cannot allocate 8589934592-byte block
fatal error: out of memory
```

**Anchors:** `runtime: out of memory:`, `fatal error: out of memory`.

### TP-6: panic with custom message

```
panic: failed to load config: open /etc/foo.conf: no such file or directory

goroutine 1 [running]:
main.loadConfig(...)
	/path/to/main.go:15
main.main()
	/path/to/main.go:8 +0x29
exit status 2
```

**Anchors:** `panic:` line-leading + stack-trace pattern.

### TP-7: hub queue retry-exhaust (bot-hq emit, current dry-run pattern)

```
[queue] Message 4242 to brian failed after 5 attempts
```

**Anchors:** literal `[queue] Message N to <agent> failed after N attempts`.

---

## 4. FP corpus — must NOT fire

These are prose shapes that should silently drop. Source: today's exhibit + projected Q&A class.

### FP-1: today's exhibit (msg 4977)

> one-sentence reply: in Go runtime, what triggers a panic vs a fatal error vs a rate-limit response from an HTTP API? Just prose, no implementation.

**Why it's prose:** no Go-runtime emit-shape; "fatal" is mid-sentence in a question. No `fatal error:` line-header, no stack-trace, no `goroutine N`.

### FP-2: design-discussion class

> The new design must handle fatal errors gracefully. We can't let a panic propagate to the parent process; rate-limit responses should retry with backoff.

**Why it's prose:** trigger-words in design-prose. No emit-shape.

### FP-3: error-reference class

> See issue #1234 — fatal error in the migration script if rate-limit hits zero.

**Why it's prose:** GitHub-issue-reference style; no actual emit.

### FP-4: code-comment class

> ```go
> // returns fatal error if config fails to load
> func loadConfig() error { ... }
> ```

**Why it's prose:** code-comment in inline-code-block, not actual emit.

### FP-5: log-config class

> Set `panic: true` in the logger config to enable strict mode.

**Why it's prose:** YAML/JSON-style config, no Go runtime context.

### FP-6: documentation-quote class

> The Go spec says: "A program that encounters a fatal error must terminate."

**Why it's prose:** quotation, possibly from docs.

### FP-7: chatter mention

> Yeah Tom mentioned fatal errors on his end too. Their queue was rate-limited.

**Why it's prose:** casual peer mention.

---

## 5. Edge cases (impl decides)

### EC-1: pasted single-line emit without surrounding context

```
fatal error: concurrent map writes
```

Just one line of actual emit, no stack-trace. Should probably FIRE (real emit, even if context-thin).

**Recommendation:** treat `fatal error:` (line-leading, colon) as anchored TP regardless of surrounding context.

### EC-2: pasted with code-fence

```
\`\`\`
fatal error: deadlock!
goroutine 1 [chan receive]:
\`\`\`
```

Inside markdown code-fence. Should FIRE (user pasting actual output, fence is markdown wrapping).

**Recommendation:** patterns operate on raw content; fence chars are inert padding.

### EC-3: log-formatted but redacted

> 2026-04-29T01:30:00Z [error] fatal: db connection lost

Has `fatal:` but in app-log format, not Go-runtime. App emit, not runtime emit. Could be either.

**Recommendation:** require `fatal error:` (with "error" word) or co-occurrence with goroutine/stack-trace anchors. App-log `fatal:` alone is too loose.

### EC-4: prose-followed-by-paste

> Hit this fatal error today:
>
> fatal error: stack overflow
> goroutine 1 [running]:

Mixed prose + paste. Should FIRE (paste-portion has anchors).

**Recommendation:** scan content for any TP-anchor; presence of prose elsewhere doesn't suppress.

---

## 6. Recommended discrimination patterns (T1.3 design proposal)

### 6.1 Replace `(?i)\bfatal\b` with shape-anchored patterns

**Drop bare `\bfatal\b`. Replace with multi-pattern set:**

```go
// fatal error in Go runtime canonical format (line-leading, colon)
regexp.MustCompile(`(?im)^fatal error:`),

// runtime emit prefix (covers stack-overflow, OOM, etc.)
regexp.MustCompile(`(?i)\bruntime:\s+(out\s+of\s+memory|goroutine\s+stack\s+exceeds)`),

// SIGSEGV in pasted signal-context (tightening current `segmentation\s+fault|SIGSEGV`)
regexp.MustCompile(`(?i)\[signal\s+SIGSEGV:`),
```

**Rationale:**
- `(?im)^fatal error:` — line-leading via `m` (multiline) flag + `^` anchor + colon. Catches TP-2/3/4/5/EC-1 + EC-2 (fence-wrapped). Drops FP-1/2/3/4/6/7 (mid-sentence "fatal"). EC-3 ambiguous — caught only if "fatal error:" exact.
- Multi-line flag is critical: TP shapes have `fatal error:` mid-paste, not at content-start. `^` without `m` flag would only match content-start.
- `runtime:` prefix catches stack-overflow + OOM heading lines that precede `fatal error:` lines.

### 6.2 Tighten other loose patterns (P1 polish, optional)

Current loose patterns that could TFP-FP via prose:

| Pattern                | Loose-ness          | Recommended tightening                                       |
| ---------------------- | ------------------- | ------------------------------------------------------------ |
| `\bfatal\b`            | bare word — FP source | (above)                                                     |
| `rate[\s\-]?limit`     | bare phrase         | optional: `(?i)(http\s+)?429\|rate[\s\-]?limit(ed)?\s+(response\|exceeded\|hit)` — anchor to canonical context. Defer if no FP exhibit yet. |
| `\bOOM\b\|out of memory` | bare              | optional: anchor to `runtime:` prefix or `Cannot allocate`. Defer if no FP. |
| `process\s+exit(ed)?`  | bare phrase         | optional: anchor to `exit status N` or `exit code N`. Defer if no FP. |
| `stack\s+overflow`     | bare phrase         | covered by `fatal error: stack overflow` pattern above       |

**Recommendation:** **T1.3 scope = `\bfatal\b` only** (the exhibit-driven case). Tighten others as Phase-K target IF FP exhibits surface. R13 SCOPE-VERIFY-PRE-DRAFT discipline: don't expand scope on speculation.

### 6.3 Test corpus structure (T1.4 B5 probe-case input)

Recommend test fixture file `internal/gemma/sentinel_test_corpus.go` (or `_test.go` with `var corpus = []TestCase{...}`):

```go
type SentinelTestCase struct {
    Name        string
    Content     string
    ExpectMatch bool   // pre-filter
    ExpectFlag  bool   // alwaysFlag
    Category    string // "TP-1", "FP-2", "EC-3", etc.
    Source      string // msg-id or "synthetic"
}

var corpus = []SentinelTestCase{
    {Name: "TP-1 panic stack", Content: TP1, ExpectMatch: true, ExpectFlag: true, Category: "TP-1"},
    {Name: "TP-2 fatal concurrent map", Content: TP2, ExpectMatch: true, ExpectFlag: true, Category: "TP-2"},
    // ...
    {Name: "FP-1 today's exhibit msg 4977", Content: FP1, ExpectMatch: false, ExpectFlag: false, Category: "FP-1", Source: "msg-4977"},
    // ...
    {Name: "EC-1 fatal-error single-line", Content: EC1, ExpectMatch: true, ExpectFlag: true, Category: "EC-1"},
    // ...
}

func TestSentinelDiscriminationCorpus(t *testing.T) {
    for _, c := range corpus {
        msg := protocol.Message{Content: c.Content}
        decision := SentinelMatch(msg)
        if decision.Match != c.ExpectMatch || decision.AlwaysFlag != c.ExpectFlag {
            t.Errorf("[%s] %s: expected match=%v flag=%v, got match=%v flag=%v",
                c.Category, c.Name, c.ExpectMatch, c.ExpectFlag, decision.Match, decision.AlwaysFlag)
        }
    }
}
```

---

## 7. Findings + recommendations summary

### F1: Today's exhibit (msg 4977) is FP-1 in this corpus
- **Driver pattern:** `(?i)\bfatal\b` (sentinel.go:44)
- **Fix:** drop bare pattern; replace with `(?im)^fatal error:` (line-leading, colon-anchored)
- **Test-lock:** add to corpus as FP-1 with `Source: "msg-4977"`; T1.4 substring-lock checks corpus passes

### F2: alwaysFlagPatterns vs preFilterPatterns asymmetry
- `\bfatal\b` is preFilter-only, but msg 4977 fired as `[HUB:FLAG:emma]`. Suggests downstream MsgFlag promotion path I haven't traced. Brian's T1.3 impl should verify path (suspect `dispatchSentinelHit` OR Emma's processing wrapper).
- **Recommendation:** trace + document; no scope-creep in T1.3 unless trace surfaces another FP-class.

### F3: Other loose patterns (rate-limit, OOM, process-exit, stack-overflow) not yet exhibited as FPs
- **Scope discipline:** T1.3 fixes `\bfatal\b` only. Tighten others when FPs surface. R13 grounds scope.

### F4: dryRunPattern queueFailPattern is well-anchored, no change needed
- Already shape-locked to bot-hq emit format `[queue] Message N to <agent> failed after N attempts`.

### F5: Test corpus structure feeds T1.4 B5 probe-case-enumeration
- Probe `sentinel-FP-on-prose` from B5 maps to corpus entry FP-1.
- Probe `sentinel-TP-on-paste` (extension) maps to TP-1/TP-2.

---

## 8. Cross-references

- **Source:** `internal/gemma/sentinel.go:42-65`
- **Exhibit:** msgs 4977/4978 (today's user-prose Go-runtime question fire)
- **Phase J spec:** `~/.bot-hq/phase/phase-j.md` §Bucket-4 + T1.3
- **Companion investigation:** `docs/plans/2026-04-29-rule-loci-audit.md` (B3a — rule loci enumeration; sentinel rules not in scope of that audit since they're not prompt-rules)
- **T1.4 input:** sentinel corpus feeds B5 probe-case enumeration

---

## 9. Status

- **Corpus complete.** Feeds T1.3 (B4) impl directly + T1.4 (B5) probe-case enumeration.
- **Next Rain investigations:** #4 B1d design-spike (now T1.5 impl precondition — high priority), #6 PB2 slash-cmd-feasibility, #7 PB3 skill-mechanism, B7 audit (markdown deliverable).
- **Blocker for T1.3:** none from this corpus. Brian's open-question (F2 alwaysFlag promotion path trace) is impl-time investigation.
