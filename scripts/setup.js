#!/usr/bin/env node

/**
 * Bot-HQ Setup Script
 *
 * Enhanced setup with CLI options for flexible configuration.
 *
 * Usage:
 *   npm run setup                    # Interactive setup
 *   npm run setup -- --help          # Show help
 *   npm run setup -- --non-interactive  # Skip prompts, use defaults
 *   npm run setup -- --port 8080     # Custom port
 *   npm run setup -- --skip-mcp      # Don't configure MCP
 *   npm run setup -- --reset         # Reset to fresh state
 */

import { existsSync, copyFileSync, writeFileSync, readFileSync, mkdirSync, chmodSync, unlinkSync, rmSync } from 'fs';
import { execSync, spawnSync } from 'child_process';
import { dirname, join } from 'path';
import { fileURLToPath } from 'url';
import { createInterface } from 'readline';

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);
const ROOT_DIR = join(__dirname, '..');

// Parse CLI arguments
const args = process.argv.slice(2);
const options = {
  help: args.includes('--help') || args.includes('-h'),
  nonInteractive: args.includes('--non-interactive') || args.includes('-y'),
  skipMcp: args.includes('--skip-mcp'),
  skipDb: args.includes('--skip-db'),
  reset: args.includes('--reset'),
  verify: args.includes('--verify'),
  port: getArgValue('--port') || '7890',
  scope: getArgValue('--scope') || process.env.BOT_HQ_SCOPE || getDefaultScope(),
};

function getArgValue(flag) {
  const idx = args.indexOf(flag);
  if (idx !== -1 && args[idx + 1] && !args[idx + 1].startsWith('-')) {
    return args[idx + 1];
  }
  return null;
}

function getDefaultScope() {
  const homeDir = process.env.HOME || process.env.USERPROFILE;
  return join(homeDir, 'Projects');
}

let rl = null;

function getReadline() {
  if (!rl) {
    rl = createInterface({
      input: process.stdin,
      output: process.stdout,
    });
  }
  return rl;
}

function closeReadline() {
  if (rl) {
    rl.close();
    rl = null;
  }
}

function ask(question, defaultValue = '') {
  if (options.nonInteractive) {
    return Promise.resolve(defaultValue);
  }
  return new Promise((resolve) => {
    const suffix = defaultValue ? ` [${defaultValue}]` : '';
    getReadline().question(`${question}${suffix}: `, (answer) => {
      resolve(answer || defaultValue);
    });
  });
}

function askYesNo(question, defaultYes = false) {
  if (options.nonInteractive) {
    return Promise.resolve(defaultYes);
  }
  return new Promise((resolve) => {
    const hint = defaultYes ? '(Y/n)' : '(y/N)';
    getReadline().question(`${question} ${hint} `, (answer) => {
      const normalized = answer.toLowerCase().trim();
      if (normalized === '') {
        resolve(defaultYes);
      } else {
        resolve(normalized === 'y' || normalized === 'yes');
      }
    });
  });
}

// Logging utilities
const colors = {
  reset: '\x1b[0m',
  bold: '\x1b[1m',
  dim: '\x1b[2m',
  cyan: '\x1b[36m',
  green: '\x1b[32m',
  yellow: '\x1b[33m',
  red: '\x1b[31m',
  blue: '\x1b[34m',
  magenta: '\x1b[35m',
};

function log(msg) {
  console.log(`${colors.cyan}[setup]${colors.reset} ${msg}`);
}

function success(msg) {
  console.log(`${colors.green}✓${colors.reset} ${msg}`);
}

function warn(msg) {
  console.log(`${colors.yellow}⚠${colors.reset} ${msg}`);
}

function error(msg) {
  console.log(`${colors.red}✗${colors.reset} ${msg}`);
}

function info(msg) {
  console.log(`${colors.blue}ℹ${colors.reset} ${msg}`);
}

function step(num, total, msg) {
  console.log(`\n${colors.magenta}[${num}/${total}]${colors.reset} ${colors.bold}${msg}${colors.reset}`);
}

function showHelp() {
  console.log(`
${colors.bold}Bot-HQ Setup${colors.reset}

Usage: npm run setup [-- options]

Options:
  -h, --help            Show this help message
  -y, --non-interactive Skip all prompts, use defaults
  --skip-mcp            Don't configure MCP server
  --skip-db             Don't initialize database
  --reset               Reset to fresh state (removes data, keeps config)
  --verify              Only verify installation, don't make changes
  --port <number>       Set custom port (default: 7890)
  --scope <path>        Set BOT_HQ_SCOPE directory (default: ~/Projects)

Examples:
  npm run setup                           # Interactive setup
  npm run setup -- -y                     # Quick setup with defaults
  npm run setup -- --port 8080            # Use custom port
  npm run setup -- --reset                # Reset database and state
  npm run setup -- --verify               # Check installation health

Environment Variables:
  BOT_HQ_SCOPE          Default working directory for agents
  BOT_HQ_PORT           Server port (overridden by --port)
  ANTHROPIC_API_KEY     Required for agent functionality
`);
}

// Prerequisite checks
function checkPrerequisites() {
  const issues = [];
  const warnings = [];

  // Check Node.js version
  const nodeVersion = process.version;
  const majorVersion = parseInt(nodeVersion.slice(1).split('.')[0], 10);
  if (majorVersion < 18) {
    issues.push(`Node.js 18+ required (found ${nodeVersion})`);
  } else if (majorVersion < 20) {
    warnings.push(`Node.js ${majorVersion} works, but 20+ recommended for best performance`);
  }

  // Check npm
  try {
    execSync('npm --version', { stdio: 'pipe' });
  } catch {
    issues.push('npm not found in PATH');
  }

  // Check Claude CLI
  try {
    const claudeVersion = execSync('claude --version 2>/dev/null || echo "not found"', {
      encoding: 'utf-8',
      stdio: ['pipe', 'pipe', 'pipe'],
    }).trim();
    if (claudeVersion === 'not found' || claudeVersion === '') {
      warnings.push('Claude CLI not found - agent functionality will be limited');
    }
  } catch {
    warnings.push('Claude CLI not found - agent functionality will be limited');
  }

  // Check git
  try {
    execSync('git --version', { stdio: 'pipe' });
  } catch {
    warnings.push('git not found - version control features will be limited');
  }

  return { issues, warnings };
}

function verifyInstallation() {
  console.log(`\n${colors.bold}=== Bot-HQ Installation Verification ===${colors.reset}\n`);

  const checks = [];

  // Check .env.local
  const envPath = join(ROOT_DIR, '.env.local');
  if (existsSync(envPath)) {
    const envContent = readFileSync(envPath, 'utf-8');
    if (envContent.includes('ANTHROPIC_API_KEY=') && !envContent.includes('your-key-here')) {
      checks.push({ name: 'Environment file', status: 'ok', detail: '.env.local configured' });
    } else {
      checks.push({ name: 'Environment file', status: 'warn', detail: 'API key not set in .env.local' });
    }
  } else {
    checks.push({ name: 'Environment file', status: 'fail', detail: '.env.local missing' });
  }

  // Check database
  const dbPath = join(ROOT_DIR, 'data', 'bot-hq.db');
  if (existsSync(dbPath)) {
    checks.push({ name: 'Database', status: 'ok', detail: 'SQLite database exists' });
  } else {
    checks.push({ name: 'Database', status: 'fail', detail: 'Database not initialized' });
  }

  // Check MCP server script
  const mcpPath = join(ROOT_DIR, 'mcp-server.sh');
  if (existsSync(mcpPath)) {
    checks.push({ name: 'MCP Server', status: 'ok', detail: 'mcp-server.sh exists' });
  } else {
    checks.push({ name: 'MCP Server', status: 'warn', detail: 'mcp-server.sh not generated' });
  }

  // Check node_modules
  const nodeModulesPath = join(ROOT_DIR, 'node_modules');
  if (existsSync(nodeModulesPath)) {
    checks.push({ name: 'Dependencies', status: 'ok', detail: 'node_modules exists' });
  } else {
    checks.push({ name: 'Dependencies', status: 'fail', detail: 'Run npm install first' });
  }

  // Check native modules
  const betterSqlitePath = join(ROOT_DIR, 'node_modules', 'better-sqlite3');
  const nodePtyPath = join(ROOT_DIR, 'node_modules', 'node-pty');
  if (existsSync(betterSqlitePath) && existsSync(nodePtyPath)) {
    checks.push({ name: 'Native modules', status: 'ok', detail: 'better-sqlite3, node-pty installed' });
  } else {
    checks.push({ name: 'Native modules', status: 'fail', detail: 'Native modules missing' });
  }

  // Check .bot-hq directory
  const botHqDir = join(process.env.HOME || '', '.bot-hq');
  if (existsSync(botHqDir)) {
    checks.push({ name: 'Bot-HQ directory', status: 'ok', detail: '~/.bot-hq exists' });
  } else {
    checks.push({ name: 'Bot-HQ directory', status: 'info', detail: 'Will be created on first run' });
  }

  // Print results
  let hasFailures = false;
  for (const check of checks) {
    let icon, color;
    switch (check.status) {
      case 'ok':
        icon = '✓';
        color = colors.green;
        break;
      case 'warn':
        icon = '⚠';
        color = colors.yellow;
        break;
      case 'fail':
        icon = '✗';
        color = colors.red;
        hasFailures = true;
        break;
      default:
        icon = 'ℹ';
        color = colors.blue;
    }
    console.log(`${color}${icon}${colors.reset} ${check.name}: ${colors.dim}${check.detail}${colors.reset}`);
  }

  console.log('');
  if (hasFailures) {
    error('Some checks failed. Run `npm run setup` to fix issues.');
    return false;
  } else {
    success('Installation looks good!');
    return true;
  }
}

async function resetInstallation() {
  console.log(`\n${colors.bold}=== Bot-HQ Reset ===${colors.reset}\n`);

  if (!options.nonInteractive) {
    const confirmed = await askYesNo('This will delete the database and reset state. Continue?', false);
    if (!confirmed) {
      info('Reset cancelled');
      return;
    }
  }

  // Remove database
  const dbPath = join(ROOT_DIR, 'data', 'bot-hq.db');
  const dbWalPath = join(ROOT_DIR, 'data', 'bot-hq.db-wal');
  const dbShmPath = join(ROOT_DIR, 'data', 'bot-hq.db-shm');

  for (const path of [dbPath, dbWalPath, dbShmPath]) {
    if (existsSync(path)) {
      unlinkSync(path);
      log(`Removed ${path}`);
    }
  }

  // Remove .bot-hq state (but keep MANAGER_PROMPT.md if customized)
  const botHqDir = join(process.env.HOME || '', '.bot-hq');
  const queuePath = join(botHqDir, 'QUEUE.md');
  const managerStatusPath = join(botHqDir, '.manager-status');
  const workspacesDir = join(botHqDir, 'workspaces');

  if (existsSync(queuePath)) {
    unlinkSync(queuePath);
    log('Removed QUEUE.md');
  }
  if (existsSync(managerStatusPath)) {
    unlinkSync(managerStatusPath);
    log('Removed .manager-status');
  }
  if (existsSync(workspacesDir)) {
    rmSync(workspacesDir, { recursive: true });
    log('Removed workspaces directory');
  }

  success('Reset complete. Run `npm run setup` to reinitialize.');
}

async function main() {
  // Handle help
  if (options.help) {
    showHelp();
    process.exit(0);
  }

  // Handle verify
  if (options.verify) {
    const ok = verifyInstallation();
    process.exit(ok ? 0 : 1);
  }

  // Handle reset
  if (options.reset) {
    await resetInstallation();
    closeReadline();
    process.exit(0);
  }

  console.log(`
${colors.bold}╔════════════════════════════════════════╗
║           Bot-HQ Setup                 ║
╚════════════════════════════════════════╝${colors.reset}
`);

  // Check prerequisites first
  step(1, 6, 'Checking prerequisites');
  const { issues, warnings } = checkPrerequisites();

  for (const issue of issues) {
    error(issue);
  }
  for (const warning of warnings) {
    warn(warning);
  }

  if (issues.length > 0) {
    console.log('');
    error('Please fix the above issues before continuing.');
    closeReadline();
    process.exit(1);
  }

  if (warnings.length > 0 && !options.nonInteractive) {
    const continueSetup = await askYesNo('Continue with warnings?', true);
    if (!continueSetup) {
      closeReadline();
      process.exit(0);
    }
  }

  success('Prerequisites OK');

  // Step 2: Environment file
  step(2, 6, 'Configuring environment');
  const envLocalPath = join(ROOT_DIR, '.env.local');
  const envExamplePath = join(ROOT_DIR, '.env.example');

  let envConfig = {
    ANTHROPIC_API_KEY: '',
    BOT_HQ_PORT: options.port,
    BOT_HQ_SCOPE: options.scope,
    BOT_HQ_URL: `http://localhost:${options.port}`,
  };

  // Load existing config if present
  if (existsSync(envLocalPath)) {
    const existing = readFileSync(envLocalPath, 'utf-8');
    for (const line of existing.split('\n')) {
      const match = line.match(/^([A-Z_]+)=(.*)$/);
      if (match) {
        envConfig[match[1]] = match[2];
      }
    }
    info('Found existing .env.local');
  }

  // Ask for API key if not set
  if (!envConfig.ANTHROPIC_API_KEY || envConfig.ANTHROPIC_API_KEY.includes('your-key-here')) {
    if (!options.nonInteractive) {
      const apiKey = await ask('Enter your Anthropic API key (or press Enter to skip)', '');
      if (apiKey) {
        envConfig.ANTHROPIC_API_KEY = apiKey;
      } else {
        envConfig.ANTHROPIC_API_KEY = 'sk-ant-api03-your-key-here';
        warn('API key not set - edit .env.local later');
      }
    } else {
      envConfig.ANTHROPIC_API_KEY = 'sk-ant-api03-your-key-here';
      warn('API key not set - edit .env.local to add it');
    }
  }

  // Ask for custom port if interactive
  if (!options.nonInteractive && !args.includes('--port')) {
    const customPort = await ask('Server port', envConfig.BOT_HQ_PORT);
    envConfig.BOT_HQ_PORT = customPort;
    envConfig.BOT_HQ_URL = `http://localhost:${customPort}`;
  }

  // Ask for scope if interactive
  if (!options.nonInteractive && !args.includes('--scope')) {
    const customScope = await ask('Projects directory (BOT_HQ_SCOPE)', envConfig.BOT_HQ_SCOPE);
    envConfig.BOT_HQ_SCOPE = customScope;
  }

  // Write .env.local
  const envContent = `# Bot-HQ Configuration
# Generated by setup script on ${new Date().toISOString()}

# Anthropic API key (required for agent functionality)
ANTHROPIC_API_KEY=${envConfig.ANTHROPIC_API_KEY}

# Server configuration
BOT_HQ_PORT=${envConfig.BOT_HQ_PORT}
BOT_HQ_URL=${envConfig.BOT_HQ_URL}

# Working directory for agents (where your projects live)
BOT_HQ_SCOPE=${envConfig.BOT_HQ_SCOPE}

# Optional: Custom shell (defaults to $SHELL or /bin/zsh)
# BOT_HQ_SHELL=/bin/zsh

# Optional: Manager iteration settings
# BOT_HQ_MAX_ITERATIONS=3
# BOT_HQ_ITERATION_DELAY=5000
`;

  writeFileSync(envLocalPath, envContent);
  success('Environment configured');

  // Step 3: Rebuild native modules
  step(3, 6, 'Building native modules');
  try {
    execSync('npm rebuild better-sqlite3 node-pty 2>&1', {
      cwd: ROOT_DIR,
      stdio: ['pipe', 'pipe', 'pipe'],
    });
    success('Native modules built');
  } catch (e) {
    warn('Native module rebuild had warnings (may still work)');
  }

  // Fix node-pty spawn-helper permissions
  const spawnHelperPaths = [
    join(ROOT_DIR, 'node_modules/node-pty/prebuilds/darwin-arm64/spawn-helper'),
    join(ROOT_DIR, 'node_modules/node-pty/prebuilds/darwin-x64/spawn-helper'),
    join(ROOT_DIR, 'node_modules/node-pty/prebuilds/linux-x64/spawn-helper'),
  ];
  for (const helperPath of spawnHelperPaths) {
    if (existsSync(helperPath)) {
      try {
        chmodSync(helperPath, '755');
      } catch {
        // Ignore errors
      }
    }
  }

  // Step 4: Create directories
  step(4, 6, 'Creating directories');
  const dataDir = join(ROOT_DIR, 'data');
  if (!existsSync(dataDir)) {
    mkdirSync(dataDir, { recursive: true });
  }
  success('Directories ready');

  // Step 5: Initialize database
  if (!options.skipDb) {
    step(5, 6, 'Initializing database');
    try {
      execSync('npm run db:push', {
        cwd: ROOT_DIR,
        stdio: ['pipe', 'pipe', 'pipe'],
        env: { ...process.env, ...envConfig },
      });
      success('Database initialized');
    } catch (e) {
      error('Database initialization failed');
      info('Try: npm rebuild better-sqlite3 && npm run db:push');
    }
  } else {
    step(5, 6, 'Skipping database (--skip-db)');
    info('Database initialization skipped');
  }

  // Step 6: MCP server configuration
  if (!options.skipMcp) {
    step(6, 6, 'Configuring MCP server');

    // Find node/npx paths
    let nodePath;
    try {
      nodePath = execSync('which node', { encoding: 'utf-8' }).trim();
    } catch {
      nodePath = '/usr/local/bin/node';
    }
    const npxPath = join(dirname(nodePath), 'npx');

    // Generate mcp-server.sh
    const mcpServerPath = join(ROOT_DIR, 'mcp-server.sh');
    const mcpServerContent = `#!/bin/bash
# Bot-HQ MCP Server
# Generated by setup script

cd "${ROOT_DIR}"
export BOT_HQ_PORT="${envConfig.BOT_HQ_PORT}"
export BOT_HQ_SCOPE="${envConfig.BOT_HQ_SCOPE}"
export BOT_HQ_URL="${envConfig.BOT_HQ_URL}"
exec "${npxPath}" tsx src/mcp/server.ts
`;

    writeFileSync(mcpServerPath, mcpServerContent);
    chmodSync(mcpServerPath, '755');
    success('Generated mcp-server.sh');

    // Ask about MCP installation
    const installMcp = await askYesNo('Install bot-hq MCP server to Claude Code?', true);

    if (installMcp) {
      // Update project-level .mcp.json
      const projectMcpPath = join(ROOT_DIR, '.mcp.json');
      let existingConfig = {};
      if (existsSync(projectMcpPath)) {
        try {
          existingConfig = JSON.parse(readFileSync(projectMcpPath, 'utf-8'));
        } catch {
          existingConfig = {};
        }
      }

      existingConfig.mcpServers = existingConfig.mcpServers || {};
      existingConfig.mcpServers['bot-hq'] = {
        command: mcpServerPath,
        args: [],
      };

      writeFileSync(projectMcpPath, JSON.stringify(existingConfig, null, 2) + '\n');
      success('Updated .mcp.json');

      console.log(`
${colors.dim}To use bot-hq MCP tools globally, add to your Claude settings:${colors.reset}

{
  "mcpServers": {
    "bot-hq": {
      "command": "${mcpServerPath}"
    }
  }
}
`);
    }
  } else {
    step(6, 6, 'Skipping MCP configuration (--skip-mcp)');
    info('MCP configuration skipped');
  }

  // Done
  closeReadline();

  console.log(`
${colors.bold}╔════════════════════════════════════════╗
║         Setup Complete! 🚀             ║
╚════════════════════════════════════════╝${colors.reset}

${colors.bold}Next steps:${colors.reset}
`);

  if (envConfig.ANTHROPIC_API_KEY.includes('your-key-here')) {
    console.log(`  1. ${colors.yellow}Edit .env.local and add your ANTHROPIC_API_KEY${colors.reset}`);
    console.log(`  2. Run: ${colors.cyan}npm run local${colors.reset}`);
  } else {
    console.log(`  1. Run: ${colors.cyan}npm run local${colors.reset}`);
  }

  console.log(`  ${envConfig.ANTHROPIC_API_KEY.includes('your-key-here') ? '3' : '2'}. Open: ${colors.cyan}http://localhost:${envConfig.BOT_HQ_PORT}${colors.reset}`);

  console.log(`
${colors.bold}Useful commands:${colors.reset}
  ${colors.dim}npm run local${colors.reset}      Start development server
  ${colors.dim}npm run setup -- --verify${colors.reset}  Check installation health
  ${colors.dim}npm run setup -- --reset${colors.reset}   Reset database and state
  ${colors.dim}npm run doctor${colors.reset}     Diagnose common issues
  ${colors.dim}npm run mcp${colors.reset}        Run MCP server standalone
`);
}

main().catch((e) => {
  error(e.message);
  if (process.env.DEBUG) {
    console.error(e);
  }
  closeReadline();
  process.exit(1);
});
