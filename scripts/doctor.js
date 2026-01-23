#!/usr/bin/env node

/**
 * Bot-HQ Doctor Script
 *
 * Diagnoses common issues and provides fixes.
 *
 * Usage:
 *   npm run doctor          # Run diagnostics
 *   npm run doctor -- --fix # Attempt to fix issues
 */

import { existsSync, readFileSync, statSync, accessSync, constants } from 'fs';
import { execSync } from 'child_process';
import { dirname, join } from 'path';
import { fileURLToPath } from 'url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);
const ROOT_DIR = join(__dirname, '..');

const args = process.argv.slice(2);
const shouldFix = args.includes('--fix');

// Colors
const colors = {
  reset: '\x1b[0m',
  bold: '\x1b[1m',
  dim: '\x1b[2m',
  cyan: '\x1b[36m',
  green: '\x1b[32m',
  yellow: '\x1b[33m',
  red: '\x1b[31m',
  blue: '\x1b[34m',
};

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

function fix(msg) {
  console.log(`${colors.cyan}→${colors.reset} ${msg}`);
}

// Diagnostic functions
const diagnostics = [
  {
    name: 'Node.js Version',
    check: () => {
      const version = process.version;
      const major = parseInt(version.slice(1).split('.')[0], 10);
      if (major < 18) {
        return { status: 'error', message: `Node.js 18+ required (found ${version})`, canFix: false };
      }
      if (major < 20) {
        return { status: 'warn', message: `Node.js ${major} works, 20+ recommended`, canFix: false };
      }
      return { status: 'ok', message: `Node.js ${version}` };
    },
  },

  {
    name: 'npm Installation',
    check: () => {
      try {
        const version = execSync('npm --version', { encoding: 'utf-8' }).trim();
        return { status: 'ok', message: `npm ${version}` };
      } catch {
        return { status: 'error', message: 'npm not found', canFix: false };
      }
    },
  },

  {
    name: 'Claude CLI',
    check: () => {
      try {
        const result = execSync('claude --version 2>/dev/null', { encoding: 'utf-8' }).trim();
        if (result) {
          return { status: 'ok', message: `Claude CLI installed` };
        }
        return { status: 'warn', message: 'Claude CLI not found - agent features limited', canFix: false };
      } catch {
        return { status: 'warn', message: 'Claude CLI not found - agent features limited', canFix: false };
      }
    },
  },

  {
    name: 'Git',
    check: () => {
      try {
        const version = execSync('git --version', { encoding: 'utf-8' }).trim();
        return { status: 'ok', message: version };
      } catch {
        return { status: 'warn', message: 'git not found', canFix: false };
      }
    },
  },

  {
    name: 'Dependencies',
    check: () => {
      const nodeModules = join(ROOT_DIR, 'node_modules');
      if (!existsSync(nodeModules)) {
        return {
          status: 'error',
          message: 'node_modules missing',
          canFix: true,
          fix: () => {
            fix('Running npm install...');
            execSync('npm install', { cwd: ROOT_DIR, stdio: 'inherit' });
          },
        };
      }
      return { status: 'ok', message: 'node_modules exists' };
    },
  },

  {
    name: 'Native Module: better-sqlite3',
    check: () => {
      const modulePath = join(ROOT_DIR, 'node_modules', 'better-sqlite3');
      if (!existsSync(modulePath)) {
        return {
          status: 'error',
          message: 'better-sqlite3 not installed',
          canFix: true,
          fix: () => {
            fix('Running npm install better-sqlite3...');
            execSync('npm install better-sqlite3', { cwd: ROOT_DIR, stdio: 'inherit' });
          },
        };
      }

      // Try to actually load it
      try {
        const testCmd = 'node -e "require(\'better-sqlite3\')"';
        execSync(testCmd, { cwd: ROOT_DIR, stdio: 'pipe' });
        return { status: 'ok', message: 'better-sqlite3 working' };
      } catch {
        return {
          status: 'error',
          message: 'better-sqlite3 binary incompatible',
          canFix: true,
          fix: () => {
            fix('Rebuilding better-sqlite3...');
            execSync('npm rebuild better-sqlite3', { cwd: ROOT_DIR, stdio: 'inherit' });
          },
        };
      }
    },
  },

  {
    name: 'Native Module: node-pty',
    check: () => {
      const modulePath = join(ROOT_DIR, 'node_modules', 'node-pty');
      if (!existsSync(modulePath)) {
        return {
          status: 'error',
          message: 'node-pty not installed',
          canFix: true,
          fix: () => {
            fix('Running npm install node-pty...');
            execSync('npm install node-pty', { cwd: ROOT_DIR, stdio: 'inherit' });
          },
        };
      }

      // Check spawn-helper permissions
      const spawnHelperPaths = [
        join(modulePath, 'prebuilds/darwin-arm64/spawn-helper'),
        join(modulePath, 'prebuilds/darwin-x64/spawn-helper'),
        join(modulePath, 'prebuilds/linux-x64/spawn-helper'),
      ];

      for (const helperPath of spawnHelperPaths) {
        if (existsSync(helperPath)) {
          try {
            accessSync(helperPath, constants.X_OK);
          } catch {
            return {
              status: 'error',
              message: 'spawn-helper missing execute permission',
              canFix: true,
              fix: () => {
                fix('Fixing spawn-helper permissions...');
                execSync(`chmod +x "${helperPath}"`);
              },
            };
          }
        }
      }

      return { status: 'ok', message: 'node-pty working' };
    },
  },

  {
    name: 'Environment File',
    check: () => {
      const envPath = join(ROOT_DIR, '.env.local');
      if (!existsSync(envPath)) {
        return {
          status: 'error',
          message: '.env.local missing',
          canFix: true,
          fix: () => {
            fix('Running setup to create .env.local...');
            execSync('npm run setup -- -y --skip-db --skip-mcp', { cwd: ROOT_DIR, stdio: 'inherit' });
          },
        };
      }

      const content = readFileSync(envPath, 'utf-8');
      if (content.includes('your-key-here')) {
        return { status: 'warn', message: 'ANTHROPIC_API_KEY not configured', canFix: false };
      }

      return { status: 'ok', message: '.env.local configured' };
    },
  },

  {
    name: 'Data Directory',
    check: () => {
      const dataDir = join(ROOT_DIR, 'data');
      if (!existsSync(dataDir)) {
        return {
          status: 'error',
          message: 'data/ directory missing',
          canFix: true,
          fix: () => {
            fix('Creating data directory...');
            require('fs').mkdirSync(dataDir, { recursive: true });
          },
        };
      }
      return { status: 'ok', message: 'data/ directory exists' };
    },
  },

  {
    name: 'Database',
    check: () => {
      const dbPath = join(ROOT_DIR, 'data', 'bot-hq.db');
      if (!existsSync(dbPath)) {
        return {
          status: 'error',
          message: 'Database not initialized',
          canFix: true,
          fix: () => {
            fix('Initializing database...');
            execSync('npm run db:push', { cwd: ROOT_DIR, stdio: 'inherit' });
          },
        };
      }

      // Check if database is valid
      try {
        const stats = statSync(dbPath);
        if (stats.size < 100) {
          return {
            status: 'error',
            message: 'Database appears corrupted (too small)',
            canFix: true,
            fix: () => {
              fix('Reinitializing database...');
              require('fs').unlinkSync(dbPath);
              execSync('npm run db:push', { cwd: ROOT_DIR, stdio: 'inherit' });
            },
          };
        }
        return { status: 'ok', message: `Database exists (${Math.round(stats.size / 1024)}KB)` };
      } catch (e) {
        return { status: 'error', message: `Database error: ${e.message}`, canFix: false };
      }
    },
  },

  {
    name: 'MCP Server Script',
    check: () => {
      const mcpPath = join(ROOT_DIR, 'mcp-server.sh');
      if (!existsSync(mcpPath)) {
        return {
          status: 'warn',
          message: 'mcp-server.sh not generated',
          canFix: true,
          fix: () => {
            fix('Running setup to generate mcp-server.sh...');
            execSync('npm run setup -- -y --skip-db', { cwd: ROOT_DIR, stdio: 'inherit' });
          },
        };
      }

      try {
        accessSync(mcpPath, constants.X_OK);
      } catch {
        return {
          status: 'error',
          message: 'mcp-server.sh not executable',
          canFix: true,
          fix: () => {
            fix('Making mcp-server.sh executable...');
            execSync(`chmod +x "${mcpPath}"`);
          },
        };
      }

      return { status: 'ok', message: 'mcp-server.sh ready' };
    },
  },

  {
    name: 'Port Availability',
    check: () => {
      // Read port from .env.local or use default
      let port = 7890;
      const envPath = join(ROOT_DIR, '.env.local');
      if (existsSync(envPath)) {
        const content = readFileSync(envPath, 'utf-8');
        const match = content.match(/BOT_HQ_PORT=(\d+)/);
        if (match) {
          port = parseInt(match[1], 10);
        }
      }

      try {
        const result = execSync(`lsof -i :${port} 2>/dev/null || true`, { encoding: 'utf-8' });
        if (result.includes('LISTEN')) {
          return { status: 'warn', message: `Port ${port} is in use (bot-hq running or conflict)`, canFix: false };
        }
        return { status: 'ok', message: `Port ${port} available` };
      } catch {
        return { status: 'ok', message: `Port ${port} likely available` };
      }
    },
  },

  {
    name: 'TypeScript Compilation',
    check: () => {
      try {
        execSync('npx tsc --noEmit 2>&1', {
          cwd: ROOT_DIR,
          encoding: 'utf-8',
          stdio: ['pipe', 'pipe', 'pipe'],
        });
        return { status: 'ok', message: 'TypeScript compiles cleanly' };
      } catch (e) {
        const output = e.stdout || e.stderr || '';
        const errorCount = (output.match(/error TS/g) || []).length;
        if (errorCount > 0) {
          return { status: 'warn', message: `${errorCount} TypeScript errors (may still work)`, canFix: false };
        }
        return { status: 'ok', message: 'TypeScript OK' };
      }
    },
  },
];

async function runDiagnostics() {
  console.log(`
${colors.bold}╔════════════════════════════════════════╗
║         Bot-HQ Doctor                  ║
╚════════════════════════════════════════╝${colors.reset}
`);

  const results = [];
  let hasErrors = false;
  let hasWarnings = false;
  let fixableCount = 0;

  for (const diagnostic of diagnostics) {
    process.stdout.write(`${colors.dim}Checking ${diagnostic.name}...${colors.reset}`);

    try {
      const result = diagnostic.check();
      results.push({ name: diagnostic.name, ...result });

      // Clear the line and show result
      process.stdout.write('\r\x1b[K');

      switch (result.status) {
        case 'ok':
          success(`${diagnostic.name}: ${result.message}`);
          break;
        case 'warn':
          warn(`${diagnostic.name}: ${result.message}`);
          hasWarnings = true;
          break;
        case 'error':
          error(`${diagnostic.name}: ${result.message}`);
          hasErrors = true;
          if (result.canFix) {
            fixableCount++;
          }
          break;
      }
    } catch (e) {
      process.stdout.write('\r\x1b[K');
      error(`${diagnostic.name}: Check failed - ${e.message}`);
      results.push({ name: diagnostic.name, status: 'error', message: e.message });
      hasErrors = true;
    }
  }

  console.log('');

  // Summary
  if (hasErrors) {
    if (fixableCount > 0) {
      info(`Found ${fixableCount} fixable issue(s).`);
      if (shouldFix) {
        console.log(`\n${colors.bold}Attempting fixes...${colors.reset}\n`);

        for (const result of results) {
          if (result.status === 'error' && result.canFix && result.fix) {
            try {
              result.fix();
              success(`Fixed: ${result.name}`);
            } catch (e) {
              error(`Could not fix ${result.name}: ${e.message}`);
            }
          }
        }

        console.log(`\n${colors.dim}Run 'npm run doctor' again to verify fixes.${colors.reset}`);
      } else {
        console.log(`Run ${colors.cyan}npm run doctor -- --fix${colors.reset} to attempt automatic fixes.`);
      }
    } else {
      error('Found issues that require manual intervention.');
    }
  } else if (hasWarnings) {
    warn('Some warnings found, but bot-hq should work.');
    console.log(`Run ${colors.cyan}npm run local${colors.reset} to start the server.`);
  } else {
    success('All checks passed! Bot-HQ is healthy.');
    console.log(`Run ${colors.cyan}npm run local${colors.reset} to start the server.`);
  }
}

runDiagnostics().catch((e) => {
  error(`Doctor failed: ${e.message}`);
  process.exit(1);
});
