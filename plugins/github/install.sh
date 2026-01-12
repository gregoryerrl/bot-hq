#!/bin/bash
set -e

PLUGIN_DIR="$HOME/.bot-hq/plugins/github"

echo "Installing GitHub plugin to $PLUGIN_DIR..."

# Create plugin directory
mkdir -p "$PLUGIN_DIR"

# Copy built files
cp -r dist "$PLUGIN_DIR/"
cp plugin.json "$PLUGIN_DIR/"
cp package.json "$PLUGIN_DIR/"

echo "GitHub plugin installed successfully!"
echo ""
echo "Next steps:"
echo "1. Restart Bot-HQ if it's running"
echo "2. Go to /plugins and enable the GitHub plugin"
echo "3. Add your GitHub Personal Access Token in plugin settings"
echo "4. Configure owner/repo in workspace settings"
