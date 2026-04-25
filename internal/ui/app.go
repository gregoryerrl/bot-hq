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
	var lastID int64
	if db != nil {
		if recent, err := db.GetRecentMessages(100); err == nil {
			// GetRecentMessages returns chronological (oldest→newest);
			// iterate forward so hubTab.messages preserves that order.
			for i := 0; i < len(recent); i++ {
				m := recent[i]
				hubTab, _ = hubTab.Update(MessageReceived{Message: m})
				if m.ID > lastID {
					lastID = m.ID
				}
			}
		}
	}
	var pane *panestate.Manager
	if db != nil {
		pane = panestate.NewManager(db, tmux.CapturePane)
	}
	agentsTab := NewAgentsTab()
	if pane != nil {
		hubTab.SetPane(pane)
		agentsTab.SetPane(pane)
	}
	return App{
		activeTab:   TabHub,
		hubTab:      hubTab,
		agentsTab:   agentsTab,
		sessionsTab: NewSessionsTab(),
		settingsTab: NewSettingsTab(cfg, db),
		db:          db,
		brian:       b,
		lastMsgID:   lastID,
		pane:        pane,
	}
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
			// Poll sessions
			if sessions, err := a.db.ListSessions(""); err == nil {
				a.sessionsTab, _ = a.sessionsTab.Update(SessionsUpdated{Sessions: sessions})
			}
			// Poll new messages
			if msgs, err := a.db.ReadMessages("", a.lastMsgID, 50); err == nil {
				for _, m := range msgs {
					a.hubTab, _ = a.hubTab.Update(MessageReceived{Message: m})
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
		if msg.Message.ID > a.lastMsgID {
			a.lastMsgID = msg.Message.ID
		}
		return a, cmd

	case AgentsUpdated:
		var cmd tea.Cmd
		a.agentsTab, cmd = a.agentsTab.Update(msg)
		return a, cmd

	case SessionsUpdated:
		var cmd tea.Cmd
		a.sessionsTab, cmd = a.sessionsTab.Update(msg)
		return a, cmd

	case SessionSelected:
		a.hubTab.SetSessionFilter(msg.SessionID)
		if msg.SessionID != "" {
			a.activeTab = TabHub
		}
		return a, nil

	case CommandSubmitted:
		// Handle slash commands
		if strings.TrimSpace(msg.Text) == "/clear" {
			a.hubTab.messages = nil
			a.hubTab.viewport.SetContent(a.hubTab.renderMessages())
			return a, nil
		}

		// Handle commands from the Hub tab command bar
		if a.db != nil {
			target, content := parseCommand(msg.Text)
			if content == "" && target != "" {
				return a, nil
			}
			m := protocol.Message{
				FromAgent: "user",
				Type:      protocol.MsgCommand,
				Content:   content,
			}
			if target != "" {
				m.ToAgent = target
			}
			a.db.InsertMessage(m)
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
		if a.activeTab == TabSettings && a.settingsTab.editing {
			if msg.String() == "ctrl+c" {
				return a, tea.Quit
			}
			var cmd tea.Cmd
			a.settingsTab, cmd = a.settingsTab.Update(msg)
			return a, cmd
		}

		switch msg.String() {
		case "ctrl+c", "q":
			return a, tea.Quit
		case "tab":
			a.activeTab = Tab((int(a.activeTab) + 1) % len(tabNames))
		case "shift+tab":
			a.activeTab = Tab((int(a.activeTab) + len(tabNames) - 1) % len(tabNames))
		case "1":
			a.activeTab = TabHub
		case "2":
			a.activeTab = TabAgents
		case "3":
			a.activeTab = TabSessions
		case "4":
			a.activeTab = TabSettings
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
