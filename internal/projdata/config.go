package projdata

import (
	"fmt"
	"os"
	"path/filepath"

	"gopkg.in/yaml.v3"
)

// projectYAMLForReadOnly is the minimal slice of a project yaml that
// projdata cares about — just `data_sources` plus the project_name
// scalar for sanity. We deliberately don't import internal/projects so
// the package is self-contained.
type projectYAMLForReadOnly struct {
	ProjectName  string              `yaml:"project_name"`
	DataSources  *ProjectDataSources `yaml:"data_sources"`
}

// canonRoot returns the canonical-store root (honors BOT_HQ_HOME).
func canonRoot() string {
	if env := os.Getenv("BOT_HQ_HOME"); env != "" {
		return env
	}
	home, _ := os.UserHomeDir()
	return filepath.Join(home, ".bot-hq")
}

// LoadConfig reads ~/.bot-hq/projects/<project>.yaml and returns the
// `data_sources` block. Missing-block-or-empty is not an error — caller
// will see an empty list and report "no data sources configured."
//
// Missing-file returns an error so a typo in project name surfaces.
func LoadConfig(project string) (*ProjectDataSources, error) {
	if project == "" {
		return nil, fmt.Errorf("project required")
	}
	path := filepath.Join(canonRoot(), "projects", project+".yaml")
	data, err := os.ReadFile(path)
	if err != nil {
		return nil, fmt.Errorf("read %s: %w", path, err)
	}
	var pcfg projectYAMLForReadOnly
	if err := yaml.Unmarshal(data, &pcfg); err != nil {
		return nil, fmt.Errorf("parse %s: %w", path, err)
	}
	if pcfg.DataSources == nil {
		return &ProjectDataSources{}, nil
	}
	return pcfg.DataSources, nil
}

// ResolveSource looks up a named source in the loaded config.
// Returns error if the name is not present (allowlist enforcement).
func ResolveSource(cfg *ProjectDataSources, name string) (DataSourceConfig, error) {
	if cfg == nil {
		return DataSourceConfig{}, fmt.Errorf("no data_sources configured")
	}
	for _, d := range cfg.Databases {
		if d.Name == name {
			return d, nil
		}
	}
	return DataSourceConfig{}, fmt.Errorf("data source %q not found in project config (configured: %d)", name, len(cfg.Databases))
}
