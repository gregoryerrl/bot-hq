import { NextResponse } from "next/server";
import { exec } from "child_process";
import { promisify } from "util";
import fs from "fs/promises";
import path from "path";
import os from "os";

const execAsync = promisify(exec);

interface ClaudeSettings {
  configPath: string;
  mcpServers: MCPServer[];
  skills: Skill[];
  plugins: Plugin[];
  marketplaces: Marketplace[];
  accountInfo: AccountInfo;
  permissions: PermissionInfo;
  globalPreferences: GlobalPreferences;
}

interface Marketplace {
  name: string;
  repo: string;
  installLocation: string;
  lastUpdated: string;
}

interface MCPServer {
  name: string;
  command: string;
  args: string[];
  env?: Record<string, string>;
  enabled: boolean;
}

interface Skill {
  name: string;
  description: string;
  location: string;
  type: "user" | "plugin";
}

interface Plugin {
  name: string;
  version: string;
  description: string;
  enabled: boolean;
}

interface AccountInfo {
  apiKeyConfigured: boolean;
  model: string;
  organization?: string;
}

interface PermissionInfo {
  autoApproveEdits: boolean;
  allowedCommands: string[];
  blockedCommands: string[];
}

interface GlobalPreferences {
  theme: string;
  verbose: boolean;
  outputFormat: string;
}

async function getClaudeConfigPath(): Promise<string> {
  const homeDir = os.homedir();
  const defaultPath = path.join(homeDir, ".claude", "settings.json");

  try {
    await fs.access(defaultPath);
    return defaultPath;
  } catch {
    return defaultPath;
  }
}

async function getMCPServers(): Promise<MCPServer[]> {
  try {
    const { stdout } = await execAsync("claude mcp list --json 2>&1", {
      timeout: 5000
    });

    const lines = stdout.trim().split("\n");
    const jsonLine = lines.find(line => line.trim().startsWith("{") || line.trim().startsWith("["));

    if (jsonLine) {
      const data = JSON.parse(jsonLine);
      if (Array.isArray(data)) {
        return data.map((server: any) => ({
          name: server.name || "Unknown",
          command: server.command || "",
          args: server.args || [],
          env: server.env,
          enabled: server.enabled !== false,
        }));
      }
    }
  } catch (error) {
    console.error("Failed to fetch MCP servers:", error);
  }

  return [];
}

async function getSkills(): Promise<Skill[]> {
  const skills: Skill[] = [];
  const homeDir = os.homedir();

  // Get user skills
  const userSkillsPath = path.join(homeDir, ".claude", "skills");
  try {
    const dirs = await fs.readdir(userSkillsPath);
    for (const dir of dirs) {
      const skillPath = path.join(userSkillsPath, dir);
      const stat = await fs.stat(skillPath);
      if (stat.isDirectory()) {
        try {
          const skillFile = path.join(skillPath, "skill.md");
          const content = await fs.readFile(skillFile, "utf-8");

          const nameMatch = content.match(/^name:\s*(.+)$/m);
          const descMatch = content.match(/^description:\s*(.+)$/m);

          skills.push({
            name: nameMatch ? nameMatch[1].trim() : dir,
            description: descMatch ? descMatch[1].trim() : "No description",
            location: skillPath,
            type: "user",
          });
        } catch {
          // Skill file not found or couldn't read
        }
      }
    }
  } catch (error) {
    console.error("Failed to read user skills:", error);
  }

  // Get plugin skills from installed plugins
  try {
    const pluginsPath = path.join(homeDir, ".claude", "plugins");
    const installedPluginsFile = path.join(pluginsPath, "installed_plugins.json");

    const pluginData = await fs.readFile(installedPluginsFile, "utf-8");
    const installedPlugins = JSON.parse(pluginData);

    for (const [pluginName, installations] of Object.entries(installedPlugins.plugins || {})) {
      const installation = (installations as any[])[0];
      if (installation) {
        const skillsPath = path.join(installation.installPath, "skills");
        try {
          const skillFiles = await fs.readdir(skillsPath);
          for (const file of skillFiles) {
            if (file.endsWith(".md")) {
              const fullPath = path.join(skillsPath, file);
              const content = await fs.readFile(fullPath, "utf-8");

              const nameMatch = content.match(/^name:\s*(.+)$/m);
              const descMatch = content.match(/^description:\s*(.+)$/m);

              skills.push({
                name: nameMatch ? nameMatch[1].trim() : file.replace(".md", ""),
                description: descMatch ? descMatch[1].trim() : "No description",
                location: fullPath,
                type: "plugin",
              });
            }
          }
        } catch {
          // Skills directory doesn't exist for this plugin
        }
      }
    }
  } catch (error) {
    console.error("Failed to read plugin skills:", error);
  }

  return skills;
}

async function getPlugins(): Promise<Plugin[]> {
  const plugins: Plugin[] = [];
  const homeDir = os.homedir();

  try {
    // Read installed plugins
    const pluginsPath = path.join(homeDir, ".claude", "plugins");
    const installedPluginsFile = path.join(pluginsPath, "installed_plugins.json");
    const settingsPath = await getClaudeConfigPath();

    const pluginData = await fs.readFile(installedPluginsFile, "utf-8");
    const installedPlugins = JSON.parse(pluginData);

    // Read enabled plugins from settings
    let enabledPlugins: Record<string, boolean> = {};
    try {
      const settingsContent = await fs.readFile(settingsPath, "utf-8");
      const settings = JSON.parse(settingsContent);
      enabledPlugins = settings.enabledPlugins || {};
    } catch {
      // Settings file doesn't exist or can't be read
    }

    for (const [pluginName, installations] of Object.entries(installedPlugins.plugins || {})) {
      const installation = (installations as any[])[0];
      if (installation) {
        // Try to read plugin.json for metadata
        let pluginDescription = "No description";
        try {
          const pluginJsonPath = path.join(installation.installPath, "plugin.json");
          const pluginJsonContent = await fs.readFile(pluginJsonPath, "utf-8");
          const pluginJson = JSON.parse(pluginJsonContent);
          pluginDescription = pluginJson.description || pluginDescription;
        } catch {
          // plugin.json doesn't exist
        }

        plugins.push({
          name: pluginName,
          version: installation.version,
          description: pluginDescription,
          enabled: enabledPlugins[pluginName] !== false,
        });
      }
    }
  } catch (error) {
    console.error("Failed to fetch plugins:", error);
  }

  return plugins;
}

async function getAccountInfo(): Promise<AccountInfo> {
  const homeDir = os.homedir();
  const apiKeyPath = path.join(homeDir, ".claude", "api_key");

  let apiKeyConfigured = false;
  try {
    await fs.access(apiKeyPath);
    apiKeyConfigured = true;
  } catch {
    const envKey = process.env.ANTHROPIC_API_KEY || process.env.CLAUDE_API_KEY;
    apiKeyConfigured = !!envKey;
  }

  return {
    apiKeyConfigured,
    model: "claude-sonnet-4-5-20250929",
    organization: undefined,
  };
}

async function getPermissions(): Promise<PermissionInfo> {
  const configPath = await getClaudeConfigPath();

  try {
    const content = await fs.readFile(configPath, "utf-8");
    const config = JSON.parse(content);

    return {
      autoApproveEdits: config.autoApproveEdits !== false,
      allowedCommands: config.allowedCommands || [],
      blockedCommands: config.blockedCommands || ["rm -rf /", "sudo rm"],
    };
  } catch {
    return {
      autoApproveEdits: true,
      allowedCommands: [],
      blockedCommands: ["rm -rf /", "sudo rm"],
    };
  }
}

async function getGlobalPreferences(): Promise<GlobalPreferences> {
  const configPath = await getClaudeConfigPath();

  try {
    const content = await fs.readFile(configPath, "utf-8");
    const config = JSON.parse(content);

    return {
      theme: config.theme || "auto",
      verbose: config.verbose === true,
      outputFormat: config.outputFormat || "text",
    };
  } catch {
    return {
      theme: "auto",
      verbose: false,
      outputFormat: "text",
    };
  }
}

async function getMarketplaces(): Promise<Marketplace[]> {
  const marketplaces: Marketplace[] = [];
  const homeDir = os.homedir();

  try {
    const pluginsPath = path.join(homeDir, ".claude", "plugins");
    const marketplacesFile = path.join(pluginsPath, "known_marketplaces.json");

    const content = await fs.readFile(marketplacesFile, "utf-8");
    const data = JSON.parse(content);

    for (const [name, info] of Object.entries(data)) {
      const marketplaceInfo = info as any;
      marketplaces.push({
        name,
        repo: marketplaceInfo.source?.repo || "Unknown",
        installLocation: marketplaceInfo.installLocation || "",
        lastUpdated: marketplaceInfo.lastUpdated || "",
      });
    }
  } catch (error) {
    console.error("Failed to read marketplaces:", error);
  }

  return marketplaces;
}

export async function GET() {
  try {
    const configPath = await getClaudeConfigPath();

    const [mcpServers, skills, plugins, marketplaces, accountInfo, permissions, globalPreferences] = await Promise.all([
      getMCPServers(),
      getSkills(),
      getPlugins(),
      getMarketplaces(),
      getAccountInfo(),
      getPermissions(),
      getGlobalPreferences(),
    ]);

    const settings: ClaudeSettings = {
      configPath,
      mcpServers,
      skills,
      plugins,
      marketplaces,
      accountInfo,
      permissions,
      globalPreferences,
    };

    return NextResponse.json(settings);
  } catch (error) {
    console.error("Failed to fetch Claude settings:", error);
    return NextResponse.json(
      { error: "Failed to fetch Claude Code settings" },
      { status: 500 }
    );
  }
}

export async function PUT(request: Request) {
  try {
    const settings: ClaudeSettings = await request.json();
    const configPath = await getClaudeConfigPath();

    let existingConfig: any = {};
    try {
      const content = await fs.readFile(configPath, "utf-8");
      existingConfig = JSON.parse(content);
    } catch {
      // File doesn't exist, will create new
    }

    // Build enabled plugins map
    const enabledPlugins: Record<string, boolean> = {};
    for (const plugin of settings.plugins) {
      enabledPlugins[plugin.name] = plugin.enabled;
    }

    const updatedConfig = {
      ...existingConfig,
      enabledPlugins,
      autoApproveEdits: settings.permissions.autoApproveEdits,
      allowedCommands: settings.permissions.allowedCommands,
      blockedCommands: settings.permissions.blockedCommands,
      theme: settings.globalPreferences.theme,
      verbose: settings.globalPreferences.verbose,
      outputFormat: settings.globalPreferences.outputFormat,
    };

    const configDir = path.dirname(configPath);
    await fs.mkdir(configDir, { recursive: true });
    await fs.writeFile(configPath, JSON.stringify(updatedConfig, null, 2), "utf-8");

    return NextResponse.json({ success: true });
  } catch (error) {
    console.error("Failed to save Claude settings:", error);
    return NextResponse.json(
      { error: "Failed to save Claude Code settings" },
      { status: 500 }
    );
  }
}
