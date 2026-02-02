# Issues and Plans

> Documentation verification findings and remediation plans for bot-hq.

**Date:** 2026-02-02
**Status:** All issues resolved

---

## Critical Issues

### 1. Fabricated External Citations

**Severity:** High
**Location:** `documentation.md` - Appendix: Credits & Inspirations

The documentation cites two GitHub repositories that do not exist:

| Claimed URL | Status |
|-------------|--------|
| `https://github.com/human-software-language/ralph-wiggum` | 404 - Not Found |
| `https://github.com/get-shit-done/gsd` | 404 - Not Found |

**Actual sources:**
- Ralph Wiggum technique:
  - https://github.com/fstandhartinger/ralph-wiggum
  - https://github.com/ghuntley/how-to-ralph-wiggum
  - https://github.com/mikeyobrien/ralph-orchestrator
- GSD (Get Shit Done):
  - https://github.com/b-r-a-n/gsd-claude
  - https://github.com/itsjwill/GSD-2.0-Get-Shit-Done-Cost-saver-

**Plan:** Update documentation.md and AGENTS.md with correct URLs or remove the citations entirely.

---

### 2. Database Schema Mismatch

**Severity:** High
**Location:** Actual database vs documented schema

The documentation claims plugin tables were "REMOVED" but they still exist in the database:

| Table | Documentation | Actual Database |
|-------|---------------|-----------------|
| `plugins` | Says removed | Exists |
| `plugin_store` | Says removed | Exists |
| `plugin_task_data` | Says removed | Exists |
| `plugin_workspace_data` | Says removed | Exists |

Additionally, `tasks` table has `source_plugin_id` column (legacy) alongside the documented `source_remote_id`.

**Plan:**
1. Create a migration to drop unused plugin tables
2. Remove `source_plugin_id` column from tasks table
3. Run `npm run db:push` to sync schema
4. Or update documentation to reflect actual state

---

### 3. No Git History for Documentation

**Severity:** Medium
**Location:** `documentation.md`

The file is untracked in git (shown as `?? documentation.md` in git status). This means:
- No authorship trail
- No change history
- No review process

**Plan:** Commit the documentation after correcting issues.

---

## Minor Issues

### 4. Undocumented API Route

**Severity:** Low
**Location:** `/api/git/diff`

The route `src/app/api/git/diff/route.ts` exists but is not documented in the API Reference section.

**Plan:** Add to documentation or remove if unused.

---

### 5. Undocumented Root Files

**Severity:** Low
**Location:** Project root

The following files are not mentioned in the file structure documentation:
- `README.md`
- `ISSUES.md`
- `next.config.ts`
- `next-env.d.ts`

**Plan:** Update file structure section to include these files.

---

## Verification Summary

| Aspect | Score | Notes |
|--------|-------|-------|
| Technical accuracy | 85% | Tech stack, config, code behavior accurate |
| Structural completeness | 70% | Missing plugin tables, undocumented route |
| External citations | 0% | Both GitHub URLs are fabricated |
| Historical provenance | N/A | No git history |
| Internal consistency | 90% | Matches AGENTS.md, README, design docs |
| Functional accuracy | 95% | Commands work as documented |

---

## Recommended Actions

### Immediate (Before Committing)

1. [x] Fix external citation URLs in `documentation.md` (Section 17 - Appendix) - DONE
2. [x] Fix external citation URLs in `AGENTS.md` (Section 2 - Inspirations) - DONE
3. [x] Add `/api/git/diff` to API Reference or remove unused route - DONE (added to API Reference)

### Short-term

4. [x] Run database migration to remove legacy plugin tables - DONE (dropped plugins, plugin_store, plugin_task_data, plugin_workspace_data)
5. [x] Update file structure section with missing root files - DONE
6. [x] Commit documentation with proper git history - DONE

### Optional

7. [ ] Consider generating documentation from code (single source of truth)
8. [ ] Add documentation linting to CI/CD

---

## Notes

The documentation is **technically accurate** for describing current codebase behavior. The main concerns are:
- Fabricated external references undermine credibility
- Database state doesn't match documented "removed" claims
- Lack of version control for the documentation itself

The techniques mentioned (Ralph Wiggum, GSD) are real and well-known in the AI agent community - only the specific URLs are incorrect.
