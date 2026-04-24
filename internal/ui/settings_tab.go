package ui

import (
	"fmt"
	"strings"

	"github.com/charmbracelet/bubbles/textinput"
	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/lipgloss"
	"github.com/gregoryerrl/bot-hq/internal/hub"
)

// SettingSaved is emitted when a setting is saved to the DB.
type SettingSaved struct {
	Key   string
	Value string
}

// SettingsTab displays configurable settings with inline editing.
type SettingsTab struct {
	config   hub.Config
	db       *hub.DB
	width    int
	height   int
	cursor   int
	editing  bool
	input    textinput.Model
	message  string // status message (e.g. "Saved!")
}

// NewSettingsTab creates a new SettingsTab with the given config and DB.
func NewSettingsTab(cfg hub.Config, db *hub.DB) SettingsTab {
	ti := textinput.New()
	ti.CharLimit = 500

	return SettingsTab{
		config: cfg,
		db:     db,
		input:  ti,
	}
}

// SetSize updates the tab's dimensions.
func (s *SettingsTab) SetSize(width, height int) {
	s.width = width
	s.height = height
}

// Update handles key events for navigation and editing.
func (s SettingsTab) Update(msg tea.Msg) (SettingsTab, tea.Cmd) {
	settings := hub.EditableSettings

	switch msg := msg.(type) {
	case tea.KeyMsg:
		if s.editing {
			switch msg.String() {
			case "enter":
				val := s.input.Value()
				key := settings[s.cursor].Key
				s.editing = false
				s.input.Blur()

				// Save to DB
				if s.db != nil {
					if err := s.db.SetSetting(key, val); err != nil {
						s.message = fmt.Sprintf("Error: %v", err)
					} else {
						// Update in-memory config
						s.applySettingToConfig(key, val)
						s.message = "Saved!"
					}
				}

				return s, func() tea.Msg {
					return SettingSaved{Key: key, Value: val}
				}
			case "esc":
				s.editing = false
				s.input.Blur()
				s.message = ""
			default:
				var cmd tea.Cmd
				s.input, cmd = s.input.Update(msg)
				return s, cmd
			}
		} else {
			switch msg.String() {
			case "j", "down":
				if s.cursor < len(settings)-1 {
					s.cursor++
					s.message = ""
				}
			case "k", "up":
				if s.cursor > 0 {
					s.cursor--
					s.message = ""
				}
			case "enter", "e":
				s.editing = true
				s.message = ""
				key := settings[s.cursor].Key
				current := s.config.GetSettingValue(key)
				s.input.SetValue(current)
				s.input.Width = s.width - 20
				return s, s.input.Focus()
			}
		}
	}

	return s, nil
}

// applySettingToConfig updates the in-memory config after a DB save.
func (s *SettingsTab) applySettingToConfig(key, val string) {
	switch key {
	case "discord.token":
		s.config.Discord.Token = val
	case "discord.channel_id":
		s.config.Discord.ChannelID = val
	case "live.gemini_api_key":
		s.config.Live.GeminiAPIKey = val
	case "live.voice":
		s.config.Live.Voice = val
	case "hub.live_port":
		fmt.Sscanf(val, "%d", &s.config.Hub.LivePort)
	case "brian.auto_start":
		s.config.Brian.AutoStart = val == "true"
	case "brian.work_dir":
		s.config.Brian.WorkDir = val
	}
}

// View renders the SettingsTab.
func (s SettingsTab) View() string {
	heading := lipgloss.NewStyle().Bold(true).Foreground(lipgloss.Color("#FFFFFF"))
	label := lipgloss.NewStyle().Foreground(ColorStatus)
	value := lipgloss.NewStyle().Foreground(lipgloss.Color("#FFFFFF"))
	selected := lipgloss.NewStyle().Foreground(lipgloss.Color("#BB77FF")).Bold(true)
	hint := lipgloss.NewStyle().Foreground(lipgloss.Color("#666666"))

	settings := hub.EditableSettings
	var b strings.Builder

	// Header with DB path (read-only)
	b.WriteString(heading.Render("HUB"))
	b.WriteString("\n")
	b.WriteString(fmt.Sprintf("  %s  %s\n",
		label.Render(fmt.Sprintf("%-14s", "Database")),
		value.Render(s.config.Hub.DBPath),
	))
	b.WriteString("\n")

	currentSection := ""
	for i, sk := range settings {
		// Section header
		if sk.Section != currentSection {
			if currentSection != "" {
				b.WriteString("\n")
			}
			currentSection = sk.Section
			b.WriteString(heading.Render(currentSection))
			b.WriteString("\n")
		}

		// Cursor indicator
		cursor := "  "
		if i == s.cursor {
			cursor = "▸ "
		}

		// Get display value
		displayVal := s.config.GetSettingValue(sk.Key)
		if sk.IsSecret {
			displayVal = maskSecret(displayVal)
		}
		if displayVal == "" {
			displayVal = "(not set)"
		}

		// Render row
		if i == s.cursor && s.editing {
			b.WriteString(fmt.Sprintf("%s%s  %s\n",
				selected.Render(cursor),
				selected.Render(fmt.Sprintf("%-14s", sk.Label)),
				s.input.View(),
			))
		} else if i == s.cursor {
			b.WriteString(fmt.Sprintf("%s%s  %s\n",
				selected.Render(cursor),
				selected.Render(fmt.Sprintf("%-14s", sk.Label)),
				selected.Render(displayVal),
			))
		} else {
			b.WriteString(fmt.Sprintf("%s%s  %s\n",
				cursor,
				label.Render(fmt.Sprintf("%-14s", sk.Label)),
				value.Render(displayVal),
			))
		}
	}

	// Status message
	if s.message != "" {
		b.WriteString("\n")
		msgStyle := lipgloss.NewStyle().Foreground(lipgloss.Color("#00FF00"))
		if strings.HasPrefix(s.message, "Error") {
			msgStyle = lipgloss.NewStyle().Foreground(lipgloss.Color("#FF0000"))
		}
		b.WriteString(msgStyle.Render("  " + s.message))
		b.WriteString("\n")
	}

	// Help line
	b.WriteString("\n")
	if s.editing {
		b.WriteString(hint.Render("  Enter: save  Esc: cancel"))
	} else {
		b.WriteString(hint.Render("  ↑/↓: navigate  Enter: edit  Tab: switch tab"))
	}

	return b.String()
}

// maskSecret replaces a secret string with dots, or shows "(not set)" if empty.
func maskSecret(s string) string {
	if s == "" {
		return "(not set)"
	}
	return strings.Repeat("●", 8)
}

// checkbox renders a boolean as a checkbox.
func checkbox(v bool) string {
	if v {
		return "[x]"
	}
	return "[ ]"
}
