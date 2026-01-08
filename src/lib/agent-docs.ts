import { getScopePath } from "@/lib/settings";
import fs from "fs/promises";
import path from "path";

const AGENT_DOCS_FOLDER = "agent-docs";

/**
 * Get the absolute path to the agent-docs directory
 */
export async function getAgentDocsPath(): Promise<string> {
  const scopePath = await getScopePath();
  return path.join(scopePath, AGENT_DOCS_FOLDER);
}

/**
 * Initialize the agent-docs folder with a README if it doesn't exist
 */
export async function initializeAgentDocs(): Promise<void> {
  const docsPath = await getAgentDocsPath();

  try {
    await fs.access(docsPath);
    // Directory exists, nothing to do
  } catch {
    // Directory doesn't exist, create it
    await fs.mkdir(docsPath, { recursive: true });

    // Create README.md
    const readmeContent = `# Agent Documentation

This folder contains documentation written by AI agents to help developers understand the codebase.

## Organization

- **Root level**: General documentation, patterns, and best practices that apply across projects
- **Subfolders**: Project-specific documentation organized by project name

## Purpose

Agents write documentation here after completing tasks to:
- Explain architectural decisions
- Document new features and how they work
- Share debugging insights
- Provide setup guides and reference materials

## Examples

\`\`\`
agent-docs/
├── README.md                    # This file
├── react-patterns.md            # General documentation
├── debugging-tips.md            # Cross-project tips
├── bot-hq/                      # Project-specific folder
│   ├── architecture.md
│   └── api-reference.md
└── other-project/
    └── setup-guide.md
\`\`\`

---

*This folder is automatically created and managed by Bot-HQ.*
`;

    const readmePath = path.join(docsPath, "README.md");
    await fs.writeFile(readmePath, readmeContent, "utf-8");

    console.log("Initialized agent-docs folder at:", docsPath);
  }
}

interface FileNode {
  path: string;
  type: "file" | "directory";
  children?: FileNode[];
}

/**
 * Recursively scan the agent-docs folder and return a tree structure
 */
export async function listDocuments(): Promise<FileNode[]> {
  const docsPath = await getAgentDocsPath();

  try {
    await fs.access(docsPath);
  } catch {
    // Folder doesn't exist, initialize it
    await initializeAgentDocs();
  }

  async function scanDirectory(dirPath: string, relativePath = ""): Promise<FileNode[]> {
    const entries = await fs.readdir(dirPath, { withFileTypes: true });
    const nodes: FileNode[] = [];

    for (const entry of entries) {
      const entryRelativePath = relativePath ? `${relativePath}/${entry.name}` : entry.name;
      const entryFullPath = path.join(dirPath, entry.name);

      if (entry.isDirectory()) {
        const children = await scanDirectory(entryFullPath, entryRelativePath);
        nodes.push({
          path: entryRelativePath,
          type: "directory",
          children: children.length > 0 ? children : undefined,
        });
      } else if (entry.isFile() && entry.name.endsWith(".md")) {
        nodes.push({
          path: entryRelativePath,
          type: "file",
        });
      }
    }

    // Sort: directories first, then files, both alphabetically
    return nodes.sort((a, b) => {
      if (a.type !== b.type) {
        return a.type === "directory" ? -1 : 1;
      }
      return a.path.localeCompare(b.path);
    });
  }

  return scanDirectory(docsPath);
}

/**
 * Read a document from the agent-docs folder
 * Validates that the path is within agent-docs (security)
 */
export async function readDocument(relativePath: string): Promise<{
  content: string;
  filename: string;
}> {
  const docsPath = await getAgentDocsPath();
  const fullPath = path.join(docsPath, relativePath);

  // Security: Ensure the resolved path is within agent-docs
  const resolvedPath = path.resolve(fullPath);
  const resolvedDocsPath = path.resolve(docsPath);

  if (!resolvedPath.startsWith(resolvedDocsPath)) {
    throw new Error("Invalid path: Access denied");
  }

  // Check if file exists and is a file
  const stats = await fs.stat(fullPath);
  if (!stats.isFile()) {
    throw new Error("Path is not a file");
  }

  // Read the file
  const content = await fs.readFile(fullPath, "utf-8");
  const filename = path.basename(fullPath);

  return { content, filename };
}

/**
 * Get the full path to a document for downloading
 */
export async function getDocumentPath(relativePath: string): Promise<string> {
  const docsPath = await getAgentDocsPath();
  const fullPath = path.join(docsPath, relativePath);

  // Security: Ensure the resolved path is within agent-docs
  const resolvedPath = path.resolve(fullPath);
  const resolvedDocsPath = path.resolve(docsPath);

  if (!resolvedPath.startsWith(resolvedDocsPath)) {
    throw new Error("Invalid path: Access denied");
  }

  // Check if file exists
  await fs.access(fullPath);

  return fullPath;
}
