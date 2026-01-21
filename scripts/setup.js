#!/usr/bin/env node

/**
 * Bot-HQ Setup Script
 *
 * Handles all first-time setup:
 * 1. Creates .env.local from .env.example
 * 2. Initializes database
 * 3. Generates mcp-server.sh with correct paths
 * 4. Optionally installs MCP server to Claude Code
 */

import { existsSync, copyFileSync, writeFileSync, readFileSync, mkdirSync, chmodSync } from 'fs';
import { execSync } from 'child_process';
import { dirname, join } from 'path';
import { fileURLToPath } from 'url';
import { createInterface } from 'readline';

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);
const ROOT_DIR = join(__dirname, '..');

const rl = createInterface({
  input: process.stdin,
  output: process.stdout,
});

function ask(question) {
  return new Promise((resolve) => {
    rl.question(question, resolve);
  });
}

function log(msg) {
  console.log(`\x1b[36m[setup]\x1b[0m ${msg}`);
}

function success(msg) {
  console.log(`\x1b[32m[setup]\x1b[0m ${msg}`);
}

function warn(msg) {
  console.log(`\x1b[33m[setup]\x1b[0m ${msg}`);
}

function error(msg) {
  console.log(`\x1b[31m[setup]\x1b[0m ${msg}`);
}

async function main() {
  console.log('\n\x1b[1m=== Bot-HQ Setup ===\x1b[0m\n');

  // Step 1: Environment file
  log('Checking environment file...');
  const envLocalPath = join(ROOT_DIR, '.env.local');
  const envExamplePath = join(ROOT_DIR, '.env.example');

  if (!existsSync(envLocalPath)) {
    if (existsSync(envExamplePath)) {
      copyFileSync(envExamplePath, envLocalPath);
      success('Created .env.local from .env.example');
      warn('Please edit .env.local and add your ANTHROPIC_API_KEY');
    } else {
      writeFileSync(envLocalPath, '# Claude API Key for agent functionality\nANTHROPIC_API_KEY=sk-ant-api03-your-key-here\n');
      success('Created .env.local template');
      warn('Please edit .env.local and add your ANTHROPIC_API_KEY');
    }
  } else {
    success('.env.local already exists');
  }

  // Step 2: Rebuild native modules (better-sqlite3, node-pty)
  log('Rebuilding native modules...');
  try {
    execSync('npm rebuild better-sqlite3 node-pty 2>/dev/null || true', { cwd: ROOT_DIR, stdio: 'inherit' });
    success('Native modules rebuilt');
  } catch {
    warn('Could not rebuild native modules (may already be correct)');
  }

  // Step 2b: Fix node-pty spawn-helper permissions (npm sometimes strips execute bit)
  const spawnHelperPaths = [
    join(ROOT_DIR, 'node_modules/node-pty/prebuilds/darwin-arm64/spawn-helper'),
    join(ROOT_DIR, 'node_modules/node-pty/prebuilds/darwin-x64/spawn-helper'),
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
  success('Fixed node-pty spawn-helper permissions');

  // Step 3: Create data directory
  const dataDir = join(ROOT_DIR, 'data');
  if (!existsSync(dataDir)) {
    mkdirSync(dataDir, { recursive: true });
    success('Created data/ directory');
  }

  // Step 4: Initialize database
  log('Initializing database...');
  try {
    execSync('npm run db:push', { cwd: ROOT_DIR, stdio: 'inherit' });
    success('Database initialized');
  } catch (e) {
    error('Failed to initialize database. Try: npm rebuild better-sqlite3 && npm run db:push');
  }

  // Step 5: Generate mcp-server.sh
  log('Generating MCP server script...');
  const mcpServerPath = join(ROOT_DIR, 'mcp-server.sh');

  // Find node path
  let nodePath;
  try {
    nodePath = execSync('which node', { encoding: 'utf-8' }).trim();
  } catch {
    nodePath = '/usr/local/bin/node';
  }

  // Find npx path (same directory as node)
  const npxPath = join(dirname(nodePath), 'npx');

  const mcpServerContent = `#!/bin/bash
cd "${ROOT_DIR}"
exec "${npxPath}" tsx src/mcp/server.ts
`;

  writeFileSync(mcpServerPath, mcpServerContent);
  chmodSync(mcpServerPath, '755');
  success('Generated mcp-server.sh');

  // Step 6: Offer to install MCP to Claude Code
  console.log('');
  const installMcp = await ask('Install bot-hq MCP server to Claude Code? (y/N) ');

  if (installMcp.toLowerCase() === 'y') {
    log('Installing MCP server to Claude Code...');

    // Check for Claude Code config locations
    const homeDir = process.env.HOME || process.env.USERPROFILE;
    const claudeConfigPaths = [
      join(homeDir, '.claude', 'claude_desktop_config.json'),
      join(homeDir, 'Library', 'Application Support', 'Claude', 'claude_desktop_config.json'),
    ];

    let configPath = null;
    for (const p of claudeConfigPaths) {
      if (existsSync(p)) {
        configPath = p;
        break;
      }
    }

    // Also try project-level .mcp.json
    const projectMcpPath = join(ROOT_DIR, '.mcp.json');

    const mcpConfig = {
      "bot-hq": {
        "command": mcpServerPath,
        "args": []
      }
    };

    // Update project-level .mcp.json
    let existingConfig = {};
    if (existsSync(projectMcpPath)) {
      try {
        existingConfig = JSON.parse(readFileSync(projectMcpPath, 'utf-8'));
      } catch {
        existingConfig = {};
      }
    }

    existingConfig.mcpServers = existingConfig.mcpServers || {};
    existingConfig.mcpServers['bot-hq'] = mcpConfig['bot-hq'];

    writeFileSync(projectMcpPath, JSON.stringify(existingConfig, null, 2) + '\n');
    success('Updated .mcp.json with bot-hq MCP server');

    console.log('\n\x1b[33mNote:\x1b[0m To use bot-hq MCP tools globally, add to your Claude settings:');
    console.log(`
{
  "mcpServers": {
    "bot-hq": {
      "command": "${mcpServerPath}",
      "args": []
    }
  }
}
`);
  }

  // Done
  rl.close();

  console.log('\n\x1b[1m=== Setup Complete ===\x1b[0m\n');
  console.log('Next steps:');
  console.log('  1. Edit .env.local with your ANTHROPIC_API_KEY');
  console.log('  2. Run: npm run local');
  console.log('  3. Open: http://localhost:7890');
  console.log('');
}

main().catch((e) => {
  error(e.message);
  process.exit(1);
});
