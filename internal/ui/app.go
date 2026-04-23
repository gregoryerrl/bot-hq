package ui

import (
	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/lipgloss"
	"github.com/gregoryerrl/bot-hq/internal/hub"
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

// App is the root Bubbletea model that manages tab navigation.
type App struct {
	activeTab   Tab
	width       int
	height      int
	hubTab      HubTab
	agentsTab   AgentsTab
	sessionsTab SessionsTab
	settingsTab SettingsTab
}

// NewApp creates a new App model with the Hub tab active.
func NewApp(cfg hub.Config) App {
	return App{
		activeTab:   TabHub,
		hubTab:      NewHubTab(),
		agentsTab:   NewAgentsTab(),
		sessionsTab: NewSessionsTab(),
		settingsTab: NewSettingsTab(cfg),
	}
}

// Init implements tea.Model.
func (a App) Init() tea.Cmd {
	return nil
}

// contentHeight returns the available height for tab content (total minus tab bar).
func (a App) contentHeight() int {
	h := a.height - 3 // Reserve space for tab bar
	if h < 1 {
		h = 1
	}
	return h
}

// Update implements tea.Model.
func (a App) Update(msg tea.Msg) (tea.Model, tea.Cmd) {
	switch msg := msg.(type) {
	case tea.WindowSizeMsg:
		a.width = msg.Width
		a.height = msg.Height
		ch := a.contentHeight()
		a.hubTab.SetSize(a.width, ch)
		a.agentsTab.SetSize(a.width, ch)
		a.sessionsTab.SetSize(a.width, ch)
		a.settingsTab.SetSize(a.width, ch)
		// Forward to hub tab so it can resize viewport
		var cmd tea.Cmd
		a.hubTab, cmd = a.hubTab.Update(msg)
		return a, cmd

	case MessageReceived:
		// Always route MessageReceived to the hub tab regardless of active tab
		var cmd tea.Cmd
		a.hubTab, cmd = a.hubTab.Update(msg)
		return a, cmd

	case AgentsUpdated:
		var cmd tea.Cmd
		a.agentsTab, cmd = a.agentsTab.Update(msg)
		return a, cmd

	case SessionsUpdated:
		var cmd tea.Cmd
		a.sessionsTab, cmd = a.sessionsTab.Update(msg)
		return a, cmd

	case tea.KeyMsg:
		// When the hub tab input is focused, route all keys there
		// except ctrl+c which always quits
		if a.activeTab == TabHub && a.hubTab.focused {
			if msg.String() == "ctrl+c" {
				return a, tea.Quit
			}
			var cmd tea.Cmd
			a.hubTab, cmd = a.hubTab.Update(msg)
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
			// Forward remaining keys to active tab
			if a.activeTab == TabHub {
				var cmd tea.Cmd
				a.hubTab, cmd = a.hubTab.Update(msg)
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
	// Render tab bar
	var tabs []string
	for i, name := range tabNames {
		if Tab(i) == a.activeTab {
			tabs = append(tabs, ActiveTabStyle.Render(" "+name+" "))
		} else {
			tabs = append(tabs, InactiveTabStyle.Render(" "+name+" "))
		}
	}
	tabBar := lipgloss.JoinHorizontal(lipgloss.Top, tabs...)
	tabBar = TabBarStyle.Width(a.width).Render(tabBar)

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
