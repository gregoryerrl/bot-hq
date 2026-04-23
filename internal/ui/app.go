package ui

import (
	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/lipgloss"
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
	activeTab Tab
	width     int
	height    int
	// Tab content will be added in later tasks
}

// NewApp creates a new App model with the Hub tab active.
func NewApp() App {
	return App{
		activeTab: TabHub,
	}
}

// Init implements tea.Model.
func (a App) Init() tea.Cmd {
	return nil
}

// Update implements tea.Model.
func (a App) Update(msg tea.Msg) (tea.Model, tea.Cmd) {
	switch msg := msg.(type) {
	case tea.KeyMsg:
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
		}
	case tea.WindowSizeMsg:
		a.width = msg.Width
		a.height = msg.Height
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

	// Content placeholder for each tab
	var content string
	switch a.activeTab {
	case TabHub:
		content = "Hub — message feed (coming soon)"
	case TabAgents:
		content = "Agents — agent list (coming soon)"
	case TabSessions:
		content = "Sessions — session list (coming soon)"
	case TabSettings:
		content = "Settings — configuration (coming soon)"
	}

	contentStyle := lipgloss.NewStyle().
		Width(a.width).
		Height(a.height - 3). // Reserve space for tab bar
		Padding(1, 2)

	return lipgloss.JoinVertical(lipgloss.Left,
		tabBar,
		contentStyle.Render(content),
	)
}
