package projdata

import (
	"fmt"
	"strings"
)

// ValidateSelectOnly rejects any query that is not a single read-only
// SELECT (or WITH-CTE-leading SELECT, or EXPLAIN of one).
//
// Rules:
//   - Strip line and block comments before parsing.
//   - Trim trailing semicolons; reject if 2+ statements present
//     (multi-statement) other than a single trailing ;.
//   - First non-whitespace keyword must be one of: SELECT, WITH, EXPLAIN.
//   - Reject if the query contains any of the mutation keywords as a
//     whole-word match at any position: INSERT, UPDATE, DELETE, DROP,
//     ALTER, CREATE, REPLACE, TRUNCATE, GRANT, REVOKE, EXEC, EXECUTE,
//     CALL, PRAGMA (writes), ATTACH, DETACH, VACUUM, ANALYZE,
//     COPY (postgres FROM/TO file is dangerous), LOAD, COMMIT, ROLLBACK,
//     BEGIN, START, SET (most SET are session-level + dangerous from
//     untrusted SQL — caller does SET RO at open-time).
//   - Reject `SELECT ... INTO <table>` (SELECT INTO is a write in PG).
//
// Conservative — false-positives can be relaxed by the caller adding
// query templates. False-negatives (allowing a write) are the failure
// class to avoid.
func ValidateSelectOnly(query string) error {
	cleaned := stripComments(query)
	cleaned = strings.TrimSpace(cleaned)
	cleaned = strings.TrimRight(cleaned, ";")
	cleaned = strings.TrimSpace(cleaned)
	if cleaned == "" {
		return fmt.Errorf("empty query")
	}
	// Reject if there's a semicolon NOT at the very end — implies multi-statement.
	if strings.Contains(cleaned, ";") {
		return fmt.Errorf("multi-statement query rejected (single SELECT only)")
	}

	upper := strings.ToUpper(cleaned)

	// First-keyword check.
	firstTok := firstKeyword(upper)
	switch firstTok {
	case "SELECT", "WITH", "EXPLAIN":
		// allowed
	default:
		return fmt.Errorf("query must begin with SELECT / WITH / EXPLAIN; got %q", firstTok)
	}

	// Banned-word scan (whole-word).
	banned := []string{
		"INSERT", "UPDATE", "DELETE", "DROP", "ALTER", "CREATE",
		"REPLACE", "TRUNCATE", "GRANT", "REVOKE", "EXEC", "EXECUTE",
		"CALL", "PRAGMA", "ATTACH", "DETACH", "VACUUM", "ANALYZE",
		"COPY", "LOAD", "COMMIT", "ROLLBACK", "BEGIN", "START", "SET",
	}
	for _, b := range banned {
		if containsWholeWord(upper, b) {
			return fmt.Errorf("query contains banned keyword %q (read-only enforcement)", b)
		}
	}

	// Reject "SELECT ... INTO <something>" (PG write form). Exclude the
	// "SELECT ... INTO TEMP" / "INSERT INTO" cases — INSERT is already
	// banned above. INTO immediately followed by another keyword like
	// OUTFILE/DUMPFILE is also dangerous.
	if containsWholeWord(upper, "INTO") {
		return fmt.Errorf("SELECT ... INTO is a write form — rejected")
	}

	return nil
}

// stripComments removes /* ... */ block comments and -- line comments
// from sql. Doesn't perfectly handle string literals containing -- or
// /*, but for read-only-gate purposes false-positives are safer than
// false-negatives.
func stripComments(s string) string {
	var b strings.Builder
	i := 0
	for i < len(s) {
		// Block comment
		if i+1 < len(s) && s[i] == '/' && s[i+1] == '*' {
			end := strings.Index(s[i+2:], "*/")
			if end < 0 {
				break // unterminated — drop the rest
			}
			i = i + 2 + end + 2
			b.WriteByte(' ')
			continue
		}
		// Line comment
		if i+1 < len(s) && s[i] == '-' && s[i+1] == '-' {
			end := strings.IndexByte(s[i:], '\n')
			if end < 0 {
				break
			}
			i = i + end
			b.WriteByte(' ')
			continue
		}
		// MySQL # line comment
		if s[i] == '#' {
			end := strings.IndexByte(s[i:], '\n')
			if end < 0 {
				break
			}
			i = i + end
			b.WriteByte(' ')
			continue
		}
		b.WriteByte(s[i])
		i++
	}
	return b.String()
}

// firstKeyword returns the first whitespace-delimited token of s,
// uppercased.
func firstKeyword(s string) string {
	s = strings.TrimSpace(s)
	if s == "" {
		return ""
	}
	end := len(s)
	for i, r := range s {
		if r == ' ' || r == '\t' || r == '\n' || r == '\r' || r == '(' {
			end = i
			break
		}
	}
	return s[:end]
}

// containsWholeWord reports whether s contains needle bounded by
// non-alphanumeric chars (so SELECTED isn't matched by SELECT).
func containsWholeWord(s, needle string) bool {
	idx := 0
	for {
		hit := strings.Index(s[idx:], needle)
		if hit < 0 {
			return false
		}
		abs := idx + hit
		// Check boundary before.
		if abs > 0 {
			c := s[abs-1]
			if isAlnum(c) || c == '_' {
				idx = abs + len(needle)
				continue
			}
		}
		// Check boundary after.
		after := abs + len(needle)
		if after < len(s) {
			c := s[after]
			if isAlnum(c) || c == '_' {
				idx = after
				continue
			}
		}
		return true
	}
}

func isAlnum(b byte) bool {
	return (b >= 'A' && b <= 'Z') || (b >= 'a' && b <= 'z') || (b >= '0' && b <= '9')
}
