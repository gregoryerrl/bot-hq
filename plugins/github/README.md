# GitHub Plugin for Bot-HQ

Integrate GitHub with Bot-HQ to sync issues, create pull requests, and manage your development workflow.

## Features

- **Sync GitHub Issues** - Import issues from a GitHub repository as Bot-HQ tasks
- **Create Pull Requests** - Automatically push branches and create PRs on approval
- **View on GitHub** - Quick links to issues and PRs from task cards

## Installation

### Build the Plugin

```bash
cd plugins/github
npm install
npm run build
```

### Install to Bot-HQ

```bash
./install.sh
```

Or manually:

```bash
mkdir -p ~/.bot-hq/plugins/github
cp -r dist plugin.json package.json ~/.bot-hq/plugins/github/
```

### Restart Bot-HQ

After installation, restart Bot-HQ to load the plugin.

## Configuration

### 1. Enable the Plugin

1. Go to **Settings > Plugins** in Bot-HQ
2. Find "GitHub" in the plugin list
3. Toggle to enable it

### 2. Add GitHub Token

1. Go to [GitHub Personal Access Tokens](https://github.com/settings/tokens)
2. Create a new token with these permissions:
   - `repo` (Full control of private repositories)
3. In Bot-HQ, go to **Settings > Plugins > GitHub**
4. Enter your token in the "GitHub Personal Access Token" field
5. Click Save

### 3. Configure Workspace

1. Go to **Settings > Workspaces**
2. Select the workspace you want to connect to GitHub
3. Under "GitHub Settings", enter:
   - **Owner**: The GitHub username or organization (e.g., `acme`)
   - **Repository**: The repository name (e.g., `widgets`)
4. Click Save

## Usage

### Syncing Issues

1. Go to the workspace page
2. Click **Sync GitHub** in the toolbar
3. Open issues from the configured repository will be imported as tasks

### Creating Pull Requests

When approving agent work:

1. Review the changes in the approval dialog
2. Check "Create Pull Request" (enabled by default)
3. Click **Accept**
4. The branch will be pushed and a PR created automatically

### Viewing on GitHub

1. Click the **...** menu on any task card
2. Select **View on GitHub**
3. The linked issue or PR will open in a new tab

## MCP Tools

The plugin provides these MCP tools for agents:

| Tool | Description |
|------|-------------|
| `github_sync_issues` | Fetch open issues from a repository |
| `github_create_pr` | Create a pull request |
| `github_push_branch` | Push a local branch to remote |
| `github_get_issue` | Get details of a specific issue |

## Settings

| Setting | Type | Default | Description |
|---------|------|---------|-------------|
| Auto-sync issues | Boolean | false | Automatically sync on startup |
| Sync interval | Number | 0 | Minutes between syncs (0 = manual) |

## Permissions Required

This plugin requires the following Bot-HQ permissions:

- `workspace:read` - Read workspace configuration
- `workspace:write` - Store GitHub repository settings
- `task:read` - Read task information
- `task:write` - Update task data with PR links
- `task:create` - Create tasks from GitHub issues
- `approval:read` - Access approval data for PR creation

## Development

### Project Structure

```
plugins/github/
├── src/
│   ├── server.ts         # MCP server with GitHub tools
│   ├── extensions.ts     # Actions and hooks
│   └── plugin-types.ts   # Type definitions
├── dist/                 # Compiled JavaScript
├── plugin.json           # Plugin manifest
├── package.json          # Dependencies
└── install.sh            # Installation script
```

### Building

```bash
npm run build
```

### Testing

```bash
npm test
```

## Troubleshooting

### "GitHub not configured for this workspace"

Make sure you've configured the owner and repository in workspace settings.

### "API rate limit exceeded"

GitHub has API rate limits. If using a personal token, you get 5,000 requests per hour. Wait for the limit to reset or use a different token.

### "Bad credentials"

Your GitHub token may be expired or invalid. Generate a new token and update it in plugin settings.

### PR Creation Fails

Ensure:
1. The token has `repo` permission
2. You have push access to the repository
3. The branch name doesn't already exist remotely

## License

MIT
