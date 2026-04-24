package hub

import (
	"fmt"
	"os"
	"path/filepath"
	"strconv"

	"github.com/BurntSushi/toml"
)

type Config struct {
	Hub     HubConfig     `toml:"hub"`
	Live    LiveConfig    `toml:"live"`
	Discord DiscordConfig `toml:"discord"`
	Brian   BrianConfig   `toml:"brian"`
	Rain    RainConfig    `toml:"rain"`
	Gemma   GemmaConfig   `toml:"gemma"`
}

type HubConfig struct {
	DBPath   string `toml:"db_path"`
	LivePort int    `toml:"live_port"`
}

type LiveConfig struct {
	Voice        string `toml:"voice"`
	GeminiAPIKey string `toml:"gemini_api_key"`
}

type DiscordConfig struct {
	Token     string `toml:"token"`
	ChannelID string `toml:"channel_id"`
}

type BrianConfig struct {
	AutoStart bool   `toml:"auto_start"`
	WorkDir   string `toml:"work_dir"`
}

type RainConfig struct {
	AutoStart bool   `toml:"auto_start"`
	WorkDir   string `toml:"work_dir"`
}

type GemmaConfig struct {
	AutoStart     bool   `toml:"auto_start"`
	Model         string `toml:"model"`
	OllamaURL     string `toml:"ollama_url"`
	MaxConcurrent int    `toml:"max_concurrent"`
}

func DefaultConfig() Config {
	home, _ := os.UserHomeDir()
	return Config{
		Hub: HubConfig{
			DBPath:   filepath.Join(home, ".bot-hq", "hub.db"),
			LivePort: 3847,
		},
		Live: LiveConfig{
			Voice: "Iapetus",
		},
	}
}

func LoadConfig(path string) (Config, error) {
	cfg := DefaultConfig()

	data, err := os.ReadFile(path)
	if err != nil {
		if os.IsNotExist(err) {
			if err := os.MkdirAll(filepath.Dir(path), 0700); err != nil {
				return cfg, err
			}
			f, err := os.OpenFile(path, os.O_CREATE|os.O_WRONLY|os.O_TRUNC, 0600)
			if err != nil {
				return cfg, err
			}
			defer f.Close()
			if err := toml.NewEncoder(f).Encode(cfg); err != nil {
				return cfg, err
			}
			return cfg, nil
		}
		return cfg, err
	}

	if _, err := toml.Decode(string(data), &cfg); err != nil {
		return cfg, err
	}

	// Override with env vars if set
	if key := os.Getenv("BOT_HQ_GEMINI_KEY"); key != "" {
		cfg.Live.GeminiAPIKey = key
	}
	if token := os.Getenv("BOT_HQ_DISCORD_TOKEN"); token != "" {
		cfg.Discord.Token = token
	}

	return cfg, nil
}

// SettingKey identifies a configurable setting.
type SettingKey struct {
	Key      string // DB key e.g. "discord.token"
	Label    string // Display label
	Section  string // Section header
	IsSecret bool   // Mask in UI
}

// EditableSettings defines which settings can be configured via the UI.
var EditableSettings = []SettingKey{
	{Key: "discord.token", Label: "Token", Section: "DISCORD", IsSecret: true},
	{Key: "discord.channel_id", Label: "Channel ID", Section: "DISCORD"},
	{Key: "live.gemini_api_key", Label: "Gemini Key", Section: "CLIVE", IsSecret: true},
	{Key: "live.voice", Label: "Voice", Section: "CLIVE"},
	{Key: "hub.live_port", Label: "Clive Port", Section: "HUB"},
	{Key: "brian.auto_start", Label: "Auto-start", Section: "BRIAN"},
	{Key: "brian.work_dir", Label: "Work Dir", Section: "BRIAN"},
	{Key: "rain.auto_start", Label: "Auto-start", Section: "RAIN"},
	{Key: "rain.work_dir", Label: "Work Dir", Section: "RAIN"},
	{Key: "gemma.auto_start", Label: "Auto-start", Section: "GEMMA"},
	{Key: "gemma.model", Label: "Model", Section: "GEMMA"},
	{Key: "gemma.ollama_url", Label: "Ollama URL", Section: "GEMMA"},
}

// ApplyDBSettings overlays DB settings onto the config.
func (cfg *Config) ApplyDBSettings(db *DB) {
	settings, err := db.GetAllSettings()
	if err != nil || len(settings) == 0 {
		return
	}

	for k, v := range settings {
		if v == "" {
			continue
		}
		switch k {
		case "discord.token":
			cfg.Discord.Token = v
		case "discord.channel_id":
			cfg.Discord.ChannelID = v
		case "live.gemini_api_key":
			cfg.Live.GeminiAPIKey = v
		case "live.voice":
			cfg.Live.Voice = v
		case "hub.live_port":
			if port, err := strconv.Atoi(v); err == nil && port > 0 {
				cfg.Hub.LivePort = port
			}
		case "brian.auto_start":
			cfg.Brian.AutoStart = v == "true"
		case "brian.work_dir":
			cfg.Brian.WorkDir = v
		case "rain.auto_start":
			cfg.Rain.AutoStart = v == "true"
		case "rain.work_dir":
			cfg.Rain.WorkDir = v
		case "gemma.auto_start":
			cfg.Gemma.AutoStart = v == "true"
		case "gemma.model":
			cfg.Gemma.Model = v
		case "gemma.ollama_url":
			cfg.Gemma.OllamaURL = v
		}
	}
}

// GetSettingValue returns the current config value for a setting key.
func (cfg *Config) GetSettingValue(key string) string {
	switch key {
	case "discord.token":
		return cfg.Discord.Token
	case "discord.channel_id":
		return cfg.Discord.ChannelID
	case "live.gemini_api_key":
		return cfg.Live.GeminiAPIKey
	case "live.voice":
		return cfg.Live.Voice
	case "hub.live_port":
		return fmt.Sprintf("%d", cfg.Hub.LivePort)
	case "brian.auto_start":
		if cfg.Brian.AutoStart {
			return "true"
		}
		return "false"
	case "brian.work_dir":
		return cfg.Brian.WorkDir
	case "rain.auto_start":
		if cfg.Rain.AutoStart {
			return "true"
		}
		return "false"
	case "rain.work_dir":
		return cfg.Rain.WorkDir
	case "gemma.auto_start":
		if cfg.Gemma.AutoStart {
			return "true"
		}
		return "false"
	case "gemma.model":
		return cfg.Gemma.Model
	case "gemma.ollama_url":
		return cfg.Gemma.OllamaURL
	default:
		return ""
	}
}
