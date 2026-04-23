package hub

import (
	"os"
	"path/filepath"

	"github.com/BurntSushi/toml"
)

type Config struct {
	Hub     HubConfig     `toml:"hub"`
	Live    LiveConfig    `toml:"live"`
	Discord DiscordConfig `toml:"discord"`
	Brain   BrainConfig   `toml:"brain"`
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

type BrainConfig struct {
	AutoStart bool `toml:"auto_start"`
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
			if err := os.MkdirAll(filepath.Dir(path), 0755); err != nil {
				return cfg, err
			}
			f, err := os.Create(path)
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
