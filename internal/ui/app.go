package ui

import (
	"strings"
	"time"

	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/lipgloss"
	"github.com/gregoryerrl/bot-hq/internal/brian"
	"github.com/gregoryerrl/bot-hq/internal/hub"
	"github.com/gregoryerrl/bot-hq/internal/panestate"
	"github.com/gregoryerrl/bot-hq/internal/protocol"
	"github.com/gregoryerrl/bot-hq/internal/tmux"
)

var tabNames = []string{"Hub", "Agents", "Sessions", "Settings"}

// Tab represents the active tab in the UI.
type Tab int

const (
	TabHub Tab = iota
	TabAgents
	TabSessions
	TabSettings
)

// tickMsg is sent periodically to poll the database.
type tickMsg time.Time

// App is the root Bubbletea model that manages tab navigation.
type App struct {
	activeTab   Tab
	width       int
	height      int
	hubTab      HubTab
	agentsTab   AgentsTab
	sessionsTab SessionsTab
	settingsTab SettingsTab
	db          *hub.DB
	brian       *brian.Brian
	lastMsgID   int64
	pane        *panestate.Manager
}

// NewApp creates a new App model with the Hub tab active.
func NewApp(cfg hub.Config, db *hub.DB, b *brian.Brian) App {
	hubTab := NewHubTab()
	sessionsTab := NewSessionsTab()
	var lastID int64
	if db != nil {
		if recent, err := db.GetRecentMessages(100); err == nil {
			// GetRecentMessages returns chronological (oldest→newest);
			// iterate forward so message order is preserved. Fan out
			// to BOTH hubTab and sessionsTab so the Z-8f container
			// view has full session history on cold boot rather than
			// starting empty (the bug observed in the Z-8 live test).
			for i := 0; i < len(recent); i++ {
				m := recent[i]
				received := MessageReceived{Message: m}
				hubTab, _ = hubTab.Update(received)
				sessionsTab, _ = sessionsTab.Update(received)
				if m.ID > lastID {
					lastID = m.ID
				}
			}
		}
		// Also seed the active session list so the Hub tab strip
		// column has content immediately rather than waiting for the
		// first tick to fire.
		if sessions, err := db.ListSessions(""); err == nil {
			upd := SessionsUpdated{Sessions: sessions}
			hubTab, _ = hubTab.Update(upd)
			sessionsTab, _ = sessionsTab.Update(upd)
		}
	}
	var pane *panestate.Manager
	if db != nil {
		pane = panestate.NewManager(db, tmux.CapturePane)
	}
	agentsTab := NewAgentsTab(tmux.CapturePane)
	if pane != nil {
		hubTab.SetPane(pane)
		agentsTab.SetPane(pane)
	}
	return App{
		activeTab:   TabHub,
		hubTab:      hubTab,
		agentsTab:   agentsTab,
		sessionsTab: sessionsTab,
		settingsTab: NewSettingsTab(cfg, db),
		db:          db,
		brian:       b,
		lastMsgID:   lastID,
		pane:        pane,
	}
}

// Pane returns the App's panestate.Manager. Nil when constructed without
// a hub.DB. Slice 5 C1 (H-32) wiring hook: cmd/bot-hq/main.go connects
// Emma's plan-usage producer to this Manager so successful 60s polls
// publish HubSnapshot{PlanUsagePct, PlanWindow} that the strip reads.
func (a App) Pane() *panestate.Manager {
	return a.pane
}

// Init implements tea.Model.
func (a App) Init() tea.Cmd {
	return tea.Batch(
		tea.EnableBracketedPaste,
		tea.Tick(time.Second, func(t time.Time) tea.Msg {
			return tickMsg(t)
		}),
	)
}

// contentHeight returns the available height for tab content (total minus tab bar).
func (a App) contentHeight() int {
	h := a.height - 2 // Reserve 2 lines for tab bar (names + border)
	if h < 1 {
		h = 1
	}
	return h
}

// Update implements tea.Model.
func (a App) Update(msg tea.Msg) (tea.Model, tea.Cmd) {
	switch msg := msg.(type) {
	case tickMsg:
		var cmds []tea.Cmd
		// Refresh agent state via panestate (single source of truth) and dispatch
		// the raw agent slice to AgentsTab so existing render paths see no change.
		// Phase E commit 4 will switch tabs to read AgentSnapshot directly.
		if a.pane != nil {
			if err := a.pane.Refresh(); err == nil {
				a.agentsTab, _ = a.agentsTab.Update(AgentsUpdated{Agents: a.pane.Agents()})
			}
		}
		if a.db != nil {
			// Poll sessions — fan out to both Sessions tab (list +
			// container) and Hub tab (Z-8e strip column).
			if sessions, err := a.db.ListSessions(""); err == nil {
				upd := SessionsUpdated{Sessions: sessions}
				a.sessionsTab, _ = a.sessionsTab.Update(upd)
				a.hubTab, _ = a.hubTab.Update(upd)
			}
			// Poll new messages — fan out to both Hub tab (main view)
			// and Sessions tab (Z-8f container stream).
			if msgs, err := a.db.ReadMessages("", a.lastMsgID, 50); err == nil {
				for _, m := range msgs {
					received := MessageReceived{Message: m}
					a.hubTab, _ = a.hubTab.Update(received)
					a.sessionsTab, _ = a.sessionsTab.Update(received)
					if m.ID > a.lastMsgID {
						a.lastMsgID = m.ID
					}
				}
			}
		}
		// Schedule next tick
		cmds = append(cmds, tea.Tick(time.Second, func(t time.Time) tea.Msg {
			return tickMsg(t)
		}))
		return a, tea.Batch(cmds...)

	case tea.WindowSizeMsg:
		a.width = msg.Width
		a.height = msg.Height
		ch := a.contentHeight()
		a.hubTab.SetSize(a.width, ch)
		a.agentsTab.SetSize(a.width, ch)
		a.sessionsTab.SetSize(a.width, ch)
		a.settingsTab.SetSize(a.width, ch)
		return a, nil

	case MessageReceived:
		// Only accept MessageReceived if it's newer than what we've seen via polling.
		// This prevents duplicates from OnMessage callbacks racing with tick polls.
		if msg.Message.ID > 0 && msg.Message.ID <= a.lastMsgID {
			return a, nil
		}
		var cmd tea.Cmd
		a.hubTab, cmd = a.hubTab.Update(msg)
		// Z-8f: also forward to Sessions tab so the drilled-in
		// container's stream stays in sync without a separate poller.
		a.sessionsTab, _ = a.sessionsTab.Update(msg)
		if msg.Message.ID > a.lastMsgID {
			a.lastMsgID = msg.Message.ID
		}
		return a, cmd

	case AgentsUpdated:
		var cmd tea.Cmd
		a.agentsTab, cmd = a.agentsTab.Update(msg)
		return a, cmd

	case SessionsUpdated:
		// Z-8e: forward to both Hub (strip column) and Sessions tab (list).
		var cmd tea.Cmd
		a.sessionsTab, cmd = a.sessionsTab.Update(msg)
		a.hubTab, _ = a.hubTab.Update(msg)
		return a, cmd

	case SessionSelected:
		// Z-8e: SessionSelected stays as a tab-routing hint. Z-8f will
		// land the Sessions-tab drilled-in container view; until then
		// it's a no-op (Sessions tab handles its own selection state
		// internally for Z-8f's container).
		return a, nil

	case CommandSubmitted:
		// /clear only affects the Hub view (main-hub chat reset).
		if strings.TrimSpace(msg.Text) == "/clear" && msg.SessionID == "" {
			a.hubTab.messages = nil
			a.hubTab.viewport.SetContent(a.hubTab.renderMessages())
			return a, nil
		}

		// Z-5i + Z-8f: ToAgent setting removed (Phase S S-4 dropped PM
		// semantics; @<target> mentions in content do the routing via
		// MentionsAgent). SessionID comes from msg — main hub submits
		// leave it "" (broadcast); Sessions tab container submits tag
		// it with the drilled-in session-id so brian/rain pollLoops in
		// that session see it and Discord forwarder routes it to the
		// session's thread (Z-7b).
		if a.db != nil {
			content := strings.TrimSpace(msg.Text)
			if content == "" {
				return a, nil
			}
			a.db.InsertMessage(protocol.Message{
				FromAgent: "user",
				Type:      protocol.MsgCommand,
				Content:   content,
				SessionID: msg.SessionID,
			})
		}
		return a, nil

	case SettingSaved:
		// Update the config in settings tab (already done internally)
		return a, nil

	case tea.KeyMsg:
		// When hub input or settings editor is focused, capture all keys
		if a.activeTab == TabHub && a.hubTab.focused {
			if msg.String() == "ctrl+c" {
				return a, tea.Quit
			}
			var cmd tea.Cmd
			a.hubTab, cmd = a.hubTab.Update(msg)
			return a, cmd
		}
		// Z-8f: container input focused — capture keys for the
		// Sessions tab so Tab-switch / typing don't escape mid-compose.
		if a.activeTab == TabSessions && a.sessionsTab.ContainerFocused() {
			if msg.String() == "ctrl+c" {
				return a, tea.Quit
			}
			var cmd tea.Cmd
			a.sessionsTab, cmd = a.sessionsTab.Update(msg)
			return a, cmd
		}
		if a.activeTab == TabSettings && a.settingsTab.editing {
			if msg.String() == "ctrl+c" {
				return a, tea.Quit
			}
			var cmd tea.Cmd
			a.settingsTab, cmd = a.settingsTab.Update(msg)
			return a, cmd
		}

		switch msg.String() {
		case "ctrl+c":
			return a, tea.Quit
		case "tab":
			a.activeTab = Tab((int(a.activeTab) + 1) % len(tabNames))
		case "shift+tab":
			a.activeTab = Tab((int(a.activeTab) + len(tabNames) - 1) % len(tabNames))
		default:
			if a.activeTab == TabHub {
				var cmd tea.Cmd
				a.hubTab, cmd = a.hubTab.Update(msg)
				return a, cmd
			}
			if a.activeTab == TabSessions {
				var cmd tea.Cmd
				a.sessionsTab, cmd = a.sessionsTab.Update(msg)
				return a, cmd
			}
			if a.activeTab == TabAgents {
				var cmd tea.Cmd
				a.agentsTab, cmd = a.agentsTab.Update(msg)
				return a, cmd
			}
			if a.activeTab == TabSettings {
				var cmd tea.Cmd
				a.settingsTab, cmd = a.settingsTab.Update(msg)
				return a, cmd
			}
		}

	case tea.MouseMsg:
		if a.activeTab == TabHub {
			var cmd tea.Cmd
			a.hubTab, cmd = a.hubTab.Update(msg)
			return a, cmd
		}
	}
	return a, nil
}

// View implements tea.Model.
func (a App) View() string {
	if a.width == 0 || a.height == 0 {
		return "Loading Bot-HQ..."
	}
	// Render tab bar
	var tabParts []string
	for i, name := range tabNames {
		if Tab(i) == a.activeTab {
			tabParts = append(tabParts, ActiveTabStyle.Render(" "+name+" "))
		} else {
			tabParts = append(tabParts, InactiveTabStyle.Render(" "+name+" "))
		}
	}
	tabLine := strings.Join(tabParts, "")
	border := lipgloss.NewStyle().Foreground(lipgloss.Color("#555555")).Render(strings.Repeat("─", a.width))
	tabBar := tabLine + "\n" + border

	// Render content for the active tab
	var content string
	switch a.activeTab {
	case TabHub:
		content = a.hubTab.View()
	case TabAgents:
		content = a.agentsTab.View()
	case TabSessions:
		content = a.sessionsTab.View()
	case TabSettings:
		content = a.settingsTab.View()
	}

	// For non-hub tabs, wrap in a styled container
	if a.activeTab != TabHub {
		contentStyle := lipgloss.NewStyle().
			Width(a.width).
			Height(a.contentHeight()).
			Padding(1, 2)
		content = contentStyle.Render(content)
	}

	return lipgloss.JoinVertical(lipgloss.Left,
		tabBar,
		content,
	)
}
