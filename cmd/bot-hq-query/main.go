// bot-hq-query is a forensics tool for the hub.db SQLite store.
//
// Phase J T1.6 (B6) — productizes the ad-hoc sqlite queries Rain ran
// during Phase I W2 hotfix investigation (msgs 4912-4924) and Phase J
// pass-3 RESUME-spam diagnosis (Rain msg 5222 hub.db trace).
//
// Subcommands:
//   - messages   List messages with filters
//   - flags      List MsgFlag emits (subset of messages)
//   - halts      Show halt_state rows (active + history)
//   - agents     List registered agents
//
// Usage:
//
//	bot-hq-query messages [--from ID] [--to ID] [--type T] [--since-id N] [--grep PATTERN] [--limit N]
//	bot-hq-query flags    [--from ID] [--since-id N] [--limit N]
//	bot-hq-query halts    [--all]
//	bot-hq-query agents
//
// Honors BOT_HQ_HOME for sqlite path (default ~/.bot-hq/hub.db).
package main

import (
	"flag"
	"fmt"
	"os"
	"path/filepath"
	"strings"

	"github.com/gregoryerrl/bot-hq/internal/hub"
)

const usage = `bot-hq-query — forensics tool for hub.db

Usage:
  bot-hq-query <subcommand> [flags]

Subcommands:
  messages   List messages with filters
  flags      List MsgFlag emits (subset of messages)
  halts      Show halt_state rows
  agents     List registered agents

Common flags:
  --since-id N   Only return rows with id > N
  --limit N      Cap result count (default 50)

messages flags:
  --from ID      Filter by sender agent_id
  --to ID        Filter by recipient agent_id
  --type T       Filter by MessageType (handshake|response|command|update|result|error|flag)
  --grep S       Substring filter on content (case-insensitive)

halts flags:
  --all          Include cleared halts (default: active only)

Env:
  BOT_HQ_HOME    Override default ~/.bot-hq path
`

func main() {
	if len(os.Args) < 2 {
		fmt.Fprint(os.Stderr, usage)
		os.Exit(2)
	}

	dbPath, err := resolveDBPath()
	if err != nil {
		fmt.Fprintf(os.Stderr, "resolve db path: %v\n", err)
		os.Exit(1)
	}
	db, err := hub.OpenDB(dbPath)
	if err != nil {
		fmt.Fprintf(os.Stderr, "open db: %v\n", err)
		os.Exit(1)
	}
	defer db.Close()

	switch os.Args[1] {
	case "messages":
		runMessages(db, os.Args[2:])
	case "flags":
		runFlags(db, os.Args[2:])
	case "halts":
		runHalts(db, os.Args[2:])
	case "agents":
		runAgents(db, os.Args[2:])
	case "-h", "--help", "help":
		fmt.Print(usage)
	default:
		fmt.Fprintf(os.Stderr, "unknown subcommand: %q\n%s", os.Args[1], usage)
		os.Exit(2)
	}
}

func resolveDBPath() (string, error) {
	if home := os.Getenv("BOT_HQ_HOME"); home != "" {
		return filepath.Join(home, "hub.db"), nil
	}
	homeDir, err := os.UserHomeDir()
	if err != nil {
		return "", err
	}
	return filepath.Join(homeDir, ".bot-hq", "hub.db"), nil
}

func runMessages(db *hub.DB, args []string) {
	fs := flag.NewFlagSet("messages", flag.ExitOnError)
	from := fs.String("from", "", "filter by sender agent_id")
	to := fs.String("to", "", "filter by recipient agent_id")
	mtype := fs.String("type", "", "filter by message type")
	sinceID := fs.Int64("since-id", 0, "only return rows with id > N")
	grep := fs.String("grep", "", "substring filter on content (case-insensitive)")
	limit := fs.Int("limit", 50, "cap result count")
	fs.Parse(args)

	msgs, err := db.GetRecentMessages(*limit + 200) // pad for client-side filter; we re-cap below
	if err != nil {
		fmt.Fprintf(os.Stderr, "query: %v\n", err)
		os.Exit(1)
	}

	grepLower := strings.ToLower(*grep)
	count := 0
	for i := len(msgs) - 1; i >= 0; i-- {
		m := msgs[i]
		if *sinceID > 0 && m.ID <= *sinceID {
			continue
		}
		if *from != "" && m.FromAgent != *from {
			continue
		}
		if *to != "" && m.ToAgent != *to {
			continue
		}
		if *mtype != "" && string(m.Type) != *mtype {
			continue
		}
		if grepLower != "" && !strings.Contains(strings.ToLower(m.Content), grepLower) {
			continue
		}
		fmt.Printf("%d\t%s\t%s\t%s→%s\t%s\n", m.ID, m.Created.Format("2006-01-02 15:04:05"), m.Type, m.FromAgent, m.ToAgent, oneline(m.Content))
		count++
		if count >= *limit {
			break
		}
	}
	if count == 0 {
		fmt.Fprintln(os.Stderr, "no rows matched")
	}
}

func runFlags(db *hub.DB, args []string) {
	// Flags = messages with type=flag. Reuse runMessages with type forced.
	args = append([]string{"--type", "flag"}, args...)
	runMessages(db, args)
}

func runHalts(db *hub.DB, args []string) {
	fs := flag.NewFlagSet("halts", flag.ExitOnError)
	all := fs.Bool("all", false, "include cleared halts")
	fs.Parse(args)

	if *all {
		fmt.Fprintln(os.Stderr, "note: ListAllHalts not yet exposed via DB API; showing active only")
	}
	for _, cause := range []string{hub.HaltCausePlanCap, hub.HaltCauseContextCap} {
		s, ok, err := db.GetHaltCause(cause)
		if err != nil {
			fmt.Fprintf(os.Stderr, "GetHaltCause(%q): %v\n", cause, err)
			continue
		}
		if !ok {
			fmt.Printf("%s\t(no active row)\n", cause)
			continue
		}
		fmt.Printf("%s\tACTIVE\tset_by=%s\tset_at=%s\treason=%s\n", cause, s.SetBy, s.SetAt.Format("2006-01-02 15:04:05"), oneline(s.Reason))
	}
	halted, err := db.IsHalted()
	if err == nil {
		fmt.Printf("\nIsHalted() = %v\n", halted)
	}
}

func runAgents(db *hub.DB, args []string) {
	fs := flag.NewFlagSet("agents", flag.ExitOnError)
	statusFilter := fs.String("status", "", "filter by status (online|working|offline)")
	fs.Parse(args)

	agents, err := db.ListAgents(*statusFilter)
	if err != nil {
		fmt.Fprintf(os.Stderr, "ListAgents: %v\n", err)
		os.Exit(1)
	}
	for _, a := range agents {
		fmt.Printf("%s\t%s\t%s\t%s\tlast_seen=%s\n", a.ID, a.Name, a.Type, a.Status, a.LastSeen.Format("2006-01-02 15:04:05"))
	}
	if len(agents) == 0 {
		fmt.Fprintln(os.Stderr, "no agents matched")
	}
}

func oneline(s string) string {
	s = strings.ReplaceAll(s, "\n", " ⏎ ")
	if len(s) > 120 {
		return s[:117] + "..."
	}
	return s
}
