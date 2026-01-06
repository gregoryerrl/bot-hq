# Phase 1: Foundation Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Set up Bot-HQ project with Next.js, SQLite database, device authorization, and basic UI shell.

**Architecture:** Next.js 14+ App Router with SQLite via Drizzle ORM. Server-side API routes handle database operations. Device auth uses token-based pairing with 6-digit codes.

**Tech Stack:** Next.js 14+, TypeScript, SQLite, Drizzle ORM, shadcn/ui, Tailwind CSS

---

## Task 1: Initialize Next.js Project

**Files:**
- Create: `/Users/gregoryerrl/Projects/bot-hq/package.json`
- Create: `/Users/gregoryerrl/Projects/bot-hq/tsconfig.json`
- Create: `/Users/gregoryerrl/Projects/bot-hq/src/app/layout.tsx`
- Create: `/Users/gregoryerrl/Projects/bot-hq/src/app/page.tsx`

**Step 1: Create Next.js project**

Run:
```bash
cd /Users/gregoryerrl/Projects/bot-hq
npx create-next-app@latest . --typescript --tailwind --eslint --app --src-dir --import-alias "@/*" --use-npm --no-turbo
```

Expected: Project scaffolded with Next.js 14+

**Step 2: Verify project runs**

Run:
```bash
cd /Users/gregoryerrl/Projects/bot-hq && npm run dev &
sleep 5
curl -s http://localhost:3000 | head -20
pkill -f "next dev"
```

Expected: HTML response from Next.js

**Step 3: Commit**

```bash
cd /Users/gregoryerrl/Projects/bot-hq
git add -A
git commit -m "feat: initialize Next.js project"
```

---

## Task 2: Install and Configure Drizzle ORM

**Files:**
- Create: `src/lib/db/schema.ts`
- Create: `src/lib/db/index.ts`
- Create: `drizzle.config.ts`
- Modify: `package.json`

**Step 1: Install Drizzle and SQLite dependencies**

Run:
```bash
cd /Users/gregoryerrl/Projects/bot-hq
npm install drizzle-orm better-sqlite3
npm install -D drizzle-kit @types/better-sqlite3
```

Expected: Packages installed

**Step 2: Create database schema**

Create `src/lib/db/schema.ts`:

```typescript
import { sqliteTable, text, integer } from "drizzle-orm/sqlite-core";

// Workspaces (repos + linked directories)
export const workspaces = sqliteTable("workspaces", {
  id: integer("id").primaryKey({ autoIncrement: true }),
  name: text("name").notNull().unique(),
  repoPath: text("repo_path").notNull(),
  githubRemote: text("github_remote"),
  linkedDirs: text("linked_dirs"), // JSON string
  buildCommand: text("build_command"),
  createdAt: integer("created_at", { mode: "timestamp" })
    .notNull()
    .$defaultFn(() => new Date()),
});

// Tasks (issues + manual tasks)
export const tasks = sqliteTable("tasks", {
  id: integer("id").primaryKey({ autoIncrement: true }),
  workspaceId: integer("workspace_id")
    .notNull()
    .references(() => workspaces.id),
  githubIssueNumber: integer("github_issue_number"),
  title: text("title").notNull(),
  description: text("description"),
  state: text("state", {
    enum: [
      "new",
      "queued",
      "analyzing",
      "plan_ready",
      "in_progress",
      "pr_draft",
      "done",
    ],
  })
    .notNull()
    .default("new"),
  priority: integer("priority").default(0),
  agentPlan: text("agent_plan"),
  branchName: text("branch_name"),
  prUrl: text("pr_url"),
  assignedAt: integer("assigned_at", { mode: "timestamp" }),
  updatedAt: integer("updated_at", { mode: "timestamp" })
    .notNull()
    .$defaultFn(() => new Date()),
});

// Pending approvals
export const approvals = sqliteTable("approvals", {
  id: integer("id").primaryKey({ autoIncrement: true }),
  taskId: integer("task_id")
    .notNull()
    .references(() => tasks.id),
  type: text("type", {
    enum: ["git_push", "external_command", "deploy"],
  }).notNull(),
  command: text("command"),
  reason: text("reason"),
  diffSummary: text("diff_summary"),
  status: text("status", {
    enum: ["pending", "approved", "rejected"],
  })
    .notNull()
    .default("pending"),
  createdAt: integer("created_at", { mode: "timestamp" })
    .notNull()
    .$defaultFn(() => new Date()),
  resolvedAt: integer("resolved_at", { mode: "timestamp" }),
});

// Logs
export const logs = sqliteTable("logs", {
  id: integer("id").primaryKey({ autoIncrement: true }),
  workspaceId: integer("workspace_id").references(() => workspaces.id),
  taskId: integer("task_id").references(() => tasks.id),
  type: text("type", {
    enum: ["agent", "test", "sync", "approval", "error", "health"],
  }).notNull(),
  message: text("message").notNull(),
  details: text("details"), // JSON string
  createdAt: integer("created_at", { mode: "timestamp" })
    .notNull()
    .$defaultFn(() => new Date()),
});

// Agent sessions
export const agentSessions = sqliteTable("agent_sessions", {
  id: integer("id").primaryKey({ autoIncrement: true }),
  workspaceId: integer("workspace_id")
    .notNull()
    .references(() => workspaces.id),
  taskId: integer("task_id").references(() => tasks.id),
  pid: integer("pid"),
  status: text("status", {
    enum: ["running", "idle", "stopped", "error"],
  })
    .notNull()
    .default("idle"),
  contextSize: integer("context_size"),
  startedAt: integer("started_at", { mode: "timestamp" }),
  lastActivityAt: integer("last_activity_at", { mode: "timestamp" }),
});

// Authorized devices
export const authorizedDevices = sqliteTable("authorized_devices", {
  id: integer("id").primaryKey({ autoIncrement: true }),
  deviceName: text("device_name").notNull(),
  deviceFingerprint: text("device_fingerprint").notNull().unique(),
  tokenHash: text("token_hash").notNull(),
  authorizedAt: integer("authorized_at", { mode: "timestamp" })
    .notNull()
    .$defaultFn(() => new Date()),
  lastSeenAt: integer("last_seen_at", { mode: "timestamp" }),
  isRevoked: integer("is_revoked", { mode: "boolean" }).notNull().default(false),
});

// Type exports
export type Workspace = typeof workspaces.$inferSelect;
export type NewWorkspace = typeof workspaces.$inferInsert;
export type Task = typeof tasks.$inferSelect;
export type NewTask = typeof tasks.$inferInsert;
export type Approval = typeof approvals.$inferSelect;
export type NewApproval = typeof approvals.$inferInsert;
export type Log = typeof logs.$inferSelect;
export type NewLog = typeof logs.$inferInsert;
export type AgentSession = typeof agentSessions.$inferSelect;
export type NewAgentSession = typeof agentSessions.$inferInsert;
export type AuthorizedDevice = typeof authorizedDevices.$inferSelect;
export type NewAuthorizedDevice = typeof authorizedDevices.$inferInsert;
```

**Step 3: Create database connection**

Create `src/lib/db/index.ts`:

```typescript
import Database from "better-sqlite3";
import { drizzle } from "drizzle-orm/better-sqlite3";
import * as schema from "./schema";
import path from "path";
import fs from "fs";

const DATA_DIR = path.join(process.cwd(), "data");

// Ensure data directory exists
if (!fs.existsSync(DATA_DIR)) {
  fs.mkdirSync(DATA_DIR, { recursive: true });
}

const sqlite = new Database(path.join(DATA_DIR, "bot-hq.db"));
sqlite.pragma("journal_mode = WAL");

export const db = drizzle(sqlite, { schema });

export * from "./schema";
```

**Step 4: Create Drizzle config**

Create `drizzle.config.ts`:

```typescript
import { defineConfig } from "drizzle-kit";

export default defineConfig({
  schema: "./src/lib/db/schema.ts",
  out: "./drizzle",
  dialect: "sqlite",
  dbCredentials: {
    url: "./data/bot-hq.db",
  },
});
```

**Step 5: Add Drizzle scripts to package.json**

Add to `package.json` scripts:
```json
{
  "scripts": {
    "db:generate": "drizzle-kit generate",
    "db:migrate": "drizzle-kit migrate",
    "db:push": "drizzle-kit push",
    "db:studio": "drizzle-kit studio"
  }
}
```

**Step 6: Generate and push schema**

Run:
```bash
cd /Users/gregoryerrl/Projects/bot-hq
npm run db:push
```

Expected: Database created with tables

**Step 7: Add data directory to gitignore**

Append to `.gitignore`:
```
# Database
data/
```

**Step 8: Commit**

```bash
cd /Users/gregoryerrl/Projects/bot-hq
git add -A
git commit -m "feat: add Drizzle ORM with SQLite schema"
```

---

## Task 3: Install and Configure shadcn/ui

**Files:**
- Create: `components.json`
- Create: `src/lib/utils.ts`
- Modify: `tailwind.config.ts`
- Modify: `src/app/globals.css`

**Step 1: Initialize shadcn/ui**

Run:
```bash
cd /Users/gregoryerrl/Projects/bot-hq
npx shadcn@latest init -d
```

Expected: shadcn/ui configured with default settings

**Step 2: Install core UI components**

Run:
```bash
cd /Users/gregoryerrl/Projects/bot-hq
npx shadcn@latest add button card badge tabs scroll-area separator input dialog alert-dialog dropdown-menu toast -y
```

Expected: Components installed to `src/components/ui/`

**Step 3: Commit**

```bash
cd /Users/gregoryerrl/Projects/bot-hq
git add -A
git commit -m "feat: add shadcn/ui components"
```

---

## Task 4: Create App Layout Shell

**Files:**
- Modify: `src/app/layout.tsx`
- Create: `src/components/layout/sidebar.tsx`
- Create: `src/components/layout/header.tsx`
- Modify: `src/app/page.tsx`

**Step 1: Create Sidebar component**

Create `src/components/layout/sidebar.tsx`:

```tsx
"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";
import { cn } from "@/lib/utils";
import { LayoutDashboard, Clock, ScrollText, Settings } from "lucide-react";

const navItems = [
  { href: "/", label: "Taskboard", icon: LayoutDashboard },
  { href: "/pending", label: "Pending", icon: Clock },
  { href: "/logs", label: "Logs", icon: ScrollText },
  { href: "/settings", label: "Settings", icon: Settings },
];

export function Sidebar() {
  const pathname = usePathname();

  return (
    <aside className="w-64 border-r bg-muted/40 p-4">
      <div className="mb-8">
        <h1 className="text-xl font-bold">Bot-HQ</h1>
        <p className="text-sm text-muted-foreground">Workflow Automation</p>
      </div>
      <nav className="space-y-1">
        {navItems.map((item) => (
          <Link
            key={item.href}
            href={item.href}
            className={cn(
              "flex items-center gap-3 rounded-md px-3 py-2 text-sm font-medium transition-colors",
              pathname === item.href
                ? "bg-primary text-primary-foreground"
                : "text-muted-foreground hover:bg-muted hover:text-foreground"
            )}
          >
            <item.icon className="h-4 w-4" />
            {item.label}
          </Link>
        ))}
      </nav>
    </aside>
  );
}
```

**Step 2: Create Header component**

Create `src/components/layout/header.tsx`:

```tsx
import { Badge } from "@/components/ui/badge";

interface HeaderProps {
  title: string;
  description?: string;
}

export function Header({ title, description }: HeaderProps) {
  return (
    <header className="border-b px-6 py-4">
      <div className="flex items-center justify-between">
        <div>
          <h2 className="text-2xl font-bold tracking-tight">{title}</h2>
          {description && (
            <p className="text-muted-foreground">{description}</p>
          )}
        </div>
        <Badge variant="outline" className="text-xs">
          Connected
        </Badge>
      </div>
    </header>
  );
}
```

**Step 3: Install lucide-react for icons**

Run:
```bash
cd /Users/gregoryerrl/Projects/bot-hq
npm install lucide-react
```

**Step 4: Update root layout**

Modify `src/app/layout.tsx`:

```tsx
import type { Metadata } from "next";
import { Inter } from "next/font/google";
import "./globals.css";
import { Sidebar } from "@/components/layout/sidebar";
import { Toaster } from "@/components/ui/toaster";

const inter = Inter({ subsets: ["latin"] });

export const metadata: Metadata = {
  title: "Bot-HQ",
  description: "Workflow Automation System",
};

export default function RootLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <html lang="en">
      <body className={inter.className}>
        <div className="flex h-screen">
          <Sidebar />
          <main className="flex-1 overflow-auto">{children}</main>
        </div>
        <Toaster />
      </body>
    </html>
  );
}
```

**Step 5: Update home page**

Modify `src/app/page.tsx`:

```tsx
import { Header } from "@/components/layout/header";

export default function TaskboardPage() {
  return (
    <div className="flex flex-col h-full">
      <Header
        title="Taskboard"
        description="Manage issues across all repositories"
      />
      <div className="flex-1 p-6">
        <div className="rounded-lg border border-dashed p-8 text-center text-muted-foreground">
          No workspaces configured. Add a workspace in Settings.
        </div>
      </div>
    </div>
  );
}
```

**Step 6: Run and verify**

Run:
```bash
cd /Users/gregoryerrl/Projects/bot-hq && npm run dev &
sleep 5
curl -s http://localhost:3000 | grep -o "Bot-HQ" | head -1
pkill -f "next dev"
```

Expected: "Bot-HQ" in output

**Step 7: Commit**

```bash
cd /Users/gregoryerrl/Projects/bot-hq
git add -A
git commit -m "feat: add app layout with sidebar and header"
```

---

## Task 5: Create Stub Pages

**Files:**
- Create: `src/app/pending/page.tsx`
- Create: `src/app/logs/page.tsx`
- Create: `src/app/settings/page.tsx`

**Step 1: Create Pending page**

Create `src/app/pending/page.tsx`:

```tsx
import { Header } from "@/components/layout/header";

export default function PendingPage() {
  return (
    <div className="flex flex-col h-full">
      <Header
        title="Pending Approvals"
        description="Actions waiting for your approval"
      />
      <div className="flex-1 p-6">
        <div className="rounded-lg border border-dashed p-8 text-center text-muted-foreground">
          No pending approvals
        </div>
      </div>
    </div>
  );
}
```

**Step 2: Create Logs page**

Create `src/app/logs/page.tsx`:

```tsx
import { Header } from "@/components/layout/header";

export default function LogsPage() {
  return (
    <div className="flex flex-col h-full">
      <Header
        title="Logs"
        description="Real-time activity stream"
      />
      <div className="flex-1 p-6">
        <div className="rounded-lg border border-dashed p-8 text-center text-muted-foreground">
          No logs yet
        </div>
      </div>
    </div>
  );
}
```

**Step 3: Create Settings page**

Create `src/app/settings/page.tsx`:

```tsx
import { Header } from "@/components/layout/header";

export default function SettingsPage() {
  return (
    <div className="flex flex-col h-full">
      <Header
        title="Settings"
        description="Configure workspaces and devices"
      />
      <div className="flex-1 p-6">
        <div className="rounded-lg border border-dashed p-8 text-center text-muted-foreground">
          Settings coming soon
        </div>
      </div>
    </div>
  );
}
```

**Step 4: Commit**

```bash
cd /Users/gregoryerrl/Projects/bot-hq
git add -A
git commit -m "feat: add stub pages for pending, logs, and settings"
```

---

## Task 6: Create Workspace API Routes

**Files:**
- Create: `src/app/api/workspaces/route.ts`
- Create: `src/app/api/workspaces/[id]/route.ts`

**Step 1: Create workspaces list/create route**

Create `src/app/api/workspaces/route.ts`:

```typescript
import { NextRequest, NextResponse } from "next/server";
import { db, workspaces, NewWorkspace } from "@/lib/db";
import { eq } from "drizzle-orm";

export async function GET() {
  try {
    const allWorkspaces = await db.select().from(workspaces);
    return NextResponse.json(allWorkspaces);
  } catch (error) {
    console.error("Failed to fetch workspaces:", error);
    return NextResponse.json(
      { error: "Failed to fetch workspaces" },
      { status: 500 }
    );
  }
}

export async function POST(request: NextRequest) {
  try {
    const body = await request.json();

    const newWorkspace: NewWorkspace = {
      name: body.name,
      repoPath: body.repoPath,
      githubRemote: body.githubRemote || null,
      linkedDirs: body.linkedDirs ? JSON.stringify(body.linkedDirs) : null,
      buildCommand: body.buildCommand || null,
    };

    const result = await db.insert(workspaces).values(newWorkspace).returning();
    return NextResponse.json(result[0], { status: 201 });
  } catch (error) {
    console.error("Failed to create workspace:", error);
    return NextResponse.json(
      { error: "Failed to create workspace" },
      { status: 500 }
    );
  }
}
```

**Step 2: Create single workspace route**

Create `src/app/api/workspaces/[id]/route.ts`:

```typescript
import { NextRequest, NextResponse } from "next/server";
import { db, workspaces } from "@/lib/db";
import { eq } from "drizzle-orm";

export async function GET(
  request: NextRequest,
  { params }: { params: Promise<{ id: string }> }
) {
  try {
    const { id } = await params;
    const workspace = await db
      .select()
      .from(workspaces)
      .where(eq(workspaces.id, parseInt(id)))
      .limit(1);

    if (workspace.length === 0) {
      return NextResponse.json(
        { error: "Workspace not found" },
        { status: 404 }
      );
    }

    return NextResponse.json(workspace[0]);
  } catch (error) {
    console.error("Failed to fetch workspace:", error);
    return NextResponse.json(
      { error: "Failed to fetch workspace" },
      { status: 500 }
    );
  }
}

export async function PATCH(
  request: NextRequest,
  { params }: { params: Promise<{ id: string }> }
) {
  try {
    const { id } = await params;
    const body = await request.json();

    const updates: Partial<typeof workspaces.$inferInsert> = {};
    if (body.name !== undefined) updates.name = body.name;
    if (body.repoPath !== undefined) updates.repoPath = body.repoPath;
    if (body.githubRemote !== undefined) updates.githubRemote = body.githubRemote;
    if (body.linkedDirs !== undefined)
      updates.linkedDirs = JSON.stringify(body.linkedDirs);
    if (body.buildCommand !== undefined) updates.buildCommand = body.buildCommand;

    const result = await db
      .update(workspaces)
      .set(updates)
      .where(eq(workspaces.id, parseInt(id)))
      .returning();

    if (result.length === 0) {
      return NextResponse.json(
        { error: "Workspace not found" },
        { status: 404 }
      );
    }

    return NextResponse.json(result[0]);
  } catch (error) {
    console.error("Failed to update workspace:", error);
    return NextResponse.json(
      { error: "Failed to update workspace" },
      { status: 500 }
    );
  }
}

export async function DELETE(
  request: NextRequest,
  { params }: { params: Promise<{ id: string }> }
) {
  try {
    const { id } = await params;
    const result = await db
      .delete(workspaces)
      .where(eq(workspaces.id, parseInt(id)))
      .returning();

    if (result.length === 0) {
      return NextResponse.json(
        { error: "Workspace not found" },
        { status: 404 }
      );
    }

    return NextResponse.json({ success: true });
  } catch (error) {
    console.error("Failed to delete workspace:", error);
    return NextResponse.json(
      { error: "Failed to delete workspace" },
      { status: 500 }
    );
  }
}
```

**Step 3: Commit**

```bash
cd /Users/gregoryerrl/Projects/bot-hq
git add -A
git commit -m "feat: add workspace API routes"
```

---

## Task 7: Create Device Authorization System

**Files:**
- Create: `src/lib/auth/index.ts`
- Create: `src/lib/auth/pairing.ts`
- Create: `src/app/api/auth/pair/route.ts`
- Create: `src/app/api/auth/verify/route.ts`

**Step 1: Create auth utilities**

Create `src/lib/auth/index.ts`:

```typescript
import crypto from "crypto";
import { db, authorizedDevices } from "@/lib/db";
import { eq, and } from "drizzle-orm";

export function generateToken(): string {
  return crypto.randomBytes(32).toString("hex");
}

export function hashToken(token: string): string {
  return crypto.createHash("sha256").update(token).digest("hex");
}

export function generatePairingCode(): string {
  return Math.floor(100000 + Math.random() * 900000).toString();
}

export async function verifyDevice(
  fingerprint: string,
  token: string
): Promise<boolean> {
  const tokenHash = hashToken(token);

  const device = await db
    .select()
    .from(authorizedDevices)
    .where(
      and(
        eq(authorizedDevices.deviceFingerprint, fingerprint),
        eq(authorizedDevices.tokenHash, tokenHash),
        eq(authorizedDevices.isRevoked, false)
      )
    )
    .limit(1);

  if (device.length > 0) {
    // Update last seen
    await db
      .update(authorizedDevices)
      .set({ lastSeenAt: new Date() })
      .where(eq(authorizedDevices.id, device[0].id));
    return true;
  }

  return false;
}

export async function authorizeDevice(
  deviceName: string,
  fingerprint: string
): Promise<string> {
  const token = generateToken();
  const tokenHash = hashToken(token);

  await db.insert(authorizedDevices).values({
    deviceName,
    deviceFingerprint: fingerprint,
    tokenHash,
  });

  return token;
}
```

**Step 2: Create pairing code manager**

Create `src/lib/auth/pairing.ts`:

```typescript
import { generatePairingCode } from "./index";

interface PendingPairing {
  code: string;
  deviceName: string;
  fingerprint: string;
  expiresAt: Date;
}

// In-memory store for pending pairings (cleared on restart)
const pendingPairings = new Map<string, PendingPairing>();

// Current active pairing code (displayed on Mac)
let activePairingSession: {
  code: string;
  expiresAt: Date;
} | null = null;

export function createPairingSession(): string {
  const code = generatePairingCode();
  activePairingSession = {
    code,
    expiresAt: new Date(Date.now() + 5 * 60 * 1000), // 5 minutes
  };
  return code;
}

export function getActivePairingCode(): string | null {
  if (!activePairingSession) return null;
  if (new Date() > activePairingSession.expiresAt) {
    activePairingSession = null;
    return null;
  }
  return activePairingSession.code;
}

export function requestPairing(
  code: string,
  deviceName: string,
  fingerprint: string
): boolean {
  if (!activePairingSession || code !== activePairingSession.code) {
    return false;
  }
  if (new Date() > activePairingSession.expiresAt) {
    activePairingSession = null;
    return false;
  }

  pendingPairings.set(fingerprint, {
    code,
    deviceName,
    fingerprint,
    expiresAt: new Date(Date.now() + 2 * 60 * 1000), // 2 minutes to approve
  });

  return true;
}

export function getPendingPairings(): PendingPairing[] {
  const now = new Date();
  const valid: PendingPairing[] = [];

  for (const [key, pairing] of pendingPairings) {
    if (now > pairing.expiresAt) {
      pendingPairings.delete(key);
    } else {
      valid.push(pairing);
    }
  }

  return valid;
}

export function approvePairing(fingerprint: string): PendingPairing | null {
  const pairing = pendingPairings.get(fingerprint);
  if (!pairing) return null;

  pendingPairings.delete(fingerprint);
  activePairingSession = null; // Clear after successful pairing

  return pairing;
}

export function rejectPairing(fingerprint: string): void {
  pendingPairings.delete(fingerprint);
}
```

**Step 3: Create pairing API route**

Create `src/app/api/auth/pair/route.ts`:

```typescript
import { NextRequest, NextResponse } from "next/server";
import {
  createPairingSession,
  getActivePairingCode,
  requestPairing,
  getPendingPairings,
  approvePairing,
  rejectPairing,
} from "@/lib/auth/pairing";
import { authorizeDevice } from "@/lib/auth";

// GET: Get current pairing code (for Mac display)
export async function GET(request: NextRequest) {
  const action = request.nextUrl.searchParams.get("action");

  if (action === "pending") {
    // Get pending pairing requests
    return NextResponse.json({ pending: getPendingPairings() });
  }

  // Get or create pairing code
  let code = getActivePairingCode();
  if (!code) {
    code = createPairingSession();
  }

  return NextResponse.json({ code });
}

// POST: Request pairing (from new device) or approve/reject (from Mac)
export async function POST(request: NextRequest) {
  const body = await request.json();
  const { action, code, deviceName, fingerprint } = body;

  if (action === "request") {
    // New device requesting pairing
    if (!code || !deviceName || !fingerprint) {
      return NextResponse.json(
        { error: "Missing required fields" },
        { status: 400 }
      );
    }

    const success = requestPairing(code, deviceName, fingerprint);
    if (!success) {
      return NextResponse.json(
        { error: "Invalid or expired code" },
        { status: 401 }
      );
    }

    return NextResponse.json({ status: "pending" });
  }

  if (action === "approve") {
    // Mac approving a device
    if (!fingerprint) {
      return NextResponse.json(
        { error: "Missing fingerprint" },
        { status: 400 }
      );
    }

    const pairing = approvePairing(fingerprint);
    if (!pairing) {
      return NextResponse.json(
        { error: "No pending pairing found" },
        { status: 404 }
      );
    }

    const token = await authorizeDevice(pairing.deviceName, pairing.fingerprint);
    return NextResponse.json({ status: "approved", token });
  }

  if (action === "reject") {
    if (!fingerprint) {
      return NextResponse.json(
        { error: "Missing fingerprint" },
        { status: 400 }
      );
    }

    rejectPairing(fingerprint);
    return NextResponse.json({ status: "rejected" });
  }

  return NextResponse.json({ error: "Invalid action" }, { status: 400 });
}
```

**Step 4: Create verify API route**

Create `src/app/api/auth/verify/route.ts`:

```typescript
import { NextRequest, NextResponse } from "next/server";
import { verifyDevice } from "@/lib/auth";

export async function POST(request: NextRequest) {
  const body = await request.json();
  const { fingerprint, token } = body;

  if (!fingerprint || !token) {
    return NextResponse.json(
      { error: "Missing credentials" },
      { status: 400 }
    );
  }

  const valid = await verifyDevice(fingerprint, token);

  if (!valid) {
    return NextResponse.json(
      { error: "Invalid or revoked credentials" },
      { status: 401 }
    );
  }

  return NextResponse.json({ valid: true });
}
```

**Step 5: Commit**

```bash
cd /Users/gregoryerrl/Projects/bot-hq
git add -A
git commit -m "feat: add device authorization system"
```

---

## Task 8: Create Settings Page with Workspace Management

**Files:**
- Modify: `src/app/settings/page.tsx`
- Create: `src/components/settings/workspace-list.tsx`
- Create: `src/components/settings/add-workspace-dialog.tsx`
- Create: `src/components/settings/device-list.tsx`
- Create: `src/components/settings/pairing-display.tsx`

**Step 1: Create workspace list component**

Create `src/components/settings/workspace-list.tsx`:

```tsx
"use client";

import { useState, useEffect } from "react";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Trash2, FolderGit2 } from "lucide-react";
import { Workspace } from "@/lib/db/schema";

interface WorkspaceListProps {
  onAddClick: () => void;
}

export function WorkspaceList({ onAddClick }: WorkspaceListProps) {
  const [workspaces, setWorkspaces] = useState<Workspace[]>([]);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    fetchWorkspaces();
  }, []);

  async function fetchWorkspaces() {
    try {
      const res = await fetch("/api/workspaces");
      const data = await res.json();
      setWorkspaces(data);
    } catch (error) {
      console.error("Failed to fetch workspaces:", error);
    } finally {
      setLoading(false);
    }
  }

  async function deleteWorkspace(id: number) {
    if (!confirm("Delete this workspace?")) return;

    try {
      await fetch(`/api/workspaces/${id}`, { method: "DELETE" });
      setWorkspaces(workspaces.filter((w) => w.id !== id));
    } catch (error) {
      console.error("Failed to delete workspace:", error);
    }
  }

  if (loading) {
    return <div className="text-muted-foreground">Loading workspaces...</div>;
  }

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <h3 className="text-lg font-semibold">Workspaces</h3>
        <Button onClick={onAddClick} size="sm">
          Add Workspace
        </Button>
      </div>

      {workspaces.length === 0 ? (
        <Card className="p-6 text-center text-muted-foreground">
          No workspaces configured
        </Card>
      ) : (
        <div className="space-y-2">
          {workspaces.map((workspace) => (
            <Card key={workspace.id} className="p-4">
              <div className="flex items-center justify-between">
                <div className="flex items-center gap-3">
                  <FolderGit2 className="h-5 w-5 text-muted-foreground" />
                  <div>
                    <div className="font-medium">{workspace.name}</div>
                    <div className="text-sm text-muted-foreground">
                      {workspace.repoPath}
                    </div>
                  </div>
                </div>
                <div className="flex items-center gap-2">
                  {workspace.githubRemote && (
                    <Badge variant="secondary">GitHub</Badge>
                  )}
                  <Button
                    variant="ghost"
                    size="icon"
                    onClick={() => deleteWorkspace(workspace.id)}
                  >
                    <Trash2 className="h-4 w-4" />
                  </Button>
                </div>
              </div>
            </Card>
          ))}
        </div>
      )}
    </div>
  );
}
```

**Step 2: Create add workspace dialog**

Create `src/components/settings/add-workspace-dialog.tsx`:

```tsx
"use client";

import { useState } from "react";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";

interface AddWorkspaceDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onSuccess: () => void;
}

export function AddWorkspaceDialog({
  open,
  onOpenChange,
  onSuccess,
}: AddWorkspaceDialogProps) {
  const [name, setName] = useState("");
  const [repoPath, setRepoPath] = useState("");
  const [githubRemote, setGithubRemote] = useState("");
  const [buildCommand, setBuildCommand] = useState("");
  const [loading, setLoading] = useState(false);

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    setLoading(true);

    try {
      const res = await fetch("/api/workspaces", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          name,
          repoPath,
          githubRemote: githubRemote || null,
          buildCommand: buildCommand || null,
        }),
      });

      if (res.ok) {
        setName("");
        setRepoPath("");
        setGithubRemote("");
        setBuildCommand("");
        onSuccess();
        onOpenChange(false);
      }
    } catch (error) {
      console.error("Failed to create workspace:", error);
    } finally {
      setLoading(false);
    }
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Add Workspace</DialogTitle>
        </DialogHeader>
        <form onSubmit={handleSubmit} className="space-y-4">
          <div className="space-y-2">
            <label className="text-sm font-medium">Name</label>
            <Input
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="my-project"
              required
            />
          </div>
          <div className="space-y-2">
            <label className="text-sm font-medium">Repository Path</label>
            <Input
              value={repoPath}
              onChange={(e) => setRepoPath(e.target.value)}
              placeholder="~/Projects/my-project"
              required
            />
          </div>
          <div className="space-y-2">
            <label className="text-sm font-medium">
              GitHub Remote (optional)
            </label>
            <Input
              value={githubRemote}
              onChange={(e) => setGithubRemote(e.target.value)}
              placeholder="owner/repo"
            />
          </div>
          <div className="space-y-2">
            <label className="text-sm font-medium">
              Build Command (optional)
            </label>
            <Input
              value={buildCommand}
              onChange={(e) => setBuildCommand(e.target.value)}
              placeholder="npm run build"
            />
          </div>
          <div className="flex justify-end gap-2">
            <Button
              type="button"
              variant="outline"
              onClick={() => onOpenChange(false)}
            >
              Cancel
            </Button>
            <Button type="submit" disabled={loading}>
              {loading ? "Adding..." : "Add Workspace"}
            </Button>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  );
}
```

**Step 3: Create device list component**

Create `src/components/settings/device-list.tsx`:

```tsx
"use client";

import { useState, useEffect } from "react";
import { Card } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Smartphone, Trash2 } from "lucide-react";
import { AuthorizedDevice } from "@/lib/db/schema";

export function DeviceList() {
  const [devices, setDevices] = useState<AuthorizedDevice[]>([]);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    fetchDevices();
  }, []);

  async function fetchDevices() {
    try {
      const res = await fetch("/api/auth/devices");
      if (res.ok) {
        const data = await res.json();
        setDevices(data);
      }
    } catch (error) {
      console.error("Failed to fetch devices:", error);
    } finally {
      setLoading(false);
    }
  }

  async function revokeDevice(id: number) {
    if (!confirm("Revoke this device?")) return;

    try {
      await fetch(`/api/auth/devices/${id}`, { method: "DELETE" });
      setDevices(devices.filter((d) => d.id !== id));
    } catch (error) {
      console.error("Failed to revoke device:", error);
    }
  }

  if (loading) {
    return <div className="text-muted-foreground">Loading devices...</div>;
  }

  return (
    <div className="space-y-4">
      <h3 className="text-lg font-semibold">Authorized Devices</h3>

      {devices.length === 0 ? (
        <Card className="p-6 text-center text-muted-foreground">
          No devices authorized
        </Card>
      ) : (
        <div className="space-y-2">
          {devices.map((device) => (
            <Card key={device.id} className="p-4">
              <div className="flex items-center justify-between">
                <div className="flex items-center gap-3">
                  <Smartphone className="h-5 w-5 text-muted-foreground" />
                  <div>
                    <div className="font-medium">{device.deviceName}</div>
                    <div className="text-sm text-muted-foreground">
                      Last seen:{" "}
                      {device.lastSeenAt
                        ? new Date(device.lastSeenAt).toLocaleDateString()
                        : "Never"}
                    </div>
                  </div>
                </div>
                <div className="flex items-center gap-2">
                  {device.isRevoked ? (
                    <Badge variant="destructive">Revoked</Badge>
                  ) : (
                    <Badge variant="secondary">Active</Badge>
                  )}
                  {!device.isRevoked && (
                    <Button
                      variant="ghost"
                      size="icon"
                      onClick={() => revokeDevice(device.id)}
                    >
                      <Trash2 className="h-4 w-4" />
                    </Button>
                  )}
                </div>
              </div>
            </Card>
          ))}
        </div>
      )}
    </div>
  );
}
```

**Step 4: Create pairing display component**

Create `src/components/settings/pairing-display.tsx`:

```tsx
"use client";

import { useState, useEffect } from "react";
import { Card } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { RefreshCw, Check, X } from "lucide-react";

interface PendingPairing {
  code: string;
  deviceName: string;
  fingerprint: string;
  expiresAt: string;
}

export function PairingDisplay() {
  const [code, setCode] = useState<string | null>(null);
  const [pending, setPending] = useState<PendingPairing[]>([]);
  const [loading, setLoading] = useState(false);

  async function fetchCode() {
    setLoading(true);
    try {
      const res = await fetch("/api/auth/pair");
      const data = await res.json();
      setCode(data.code);
    } catch (error) {
      console.error("Failed to fetch pairing code:", error);
    } finally {
      setLoading(false);
    }
  }

  async function fetchPending() {
    try {
      const res = await fetch("/api/auth/pair?action=pending");
      const data = await res.json();
      setPending(data.pending || []);
    } catch (error) {
      console.error("Failed to fetch pending:", error);
    }
  }

  async function handleApprove(fingerprint: string) {
    try {
      await fetch("/api/auth/pair", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ action: "approve", fingerprint }),
      });
      fetchPending();
    } catch (error) {
      console.error("Failed to approve:", error);
    }
  }

  async function handleReject(fingerprint: string) {
    try {
      await fetch("/api/auth/pair", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ action: "reject", fingerprint }),
      });
      fetchPending();
    } catch (error) {
      console.error("Failed to reject:", error);
    }
  }

  useEffect(() => {
    fetchCode();
    fetchPending();
    const interval = setInterval(fetchPending, 5000);
    return () => clearInterval(interval);
  }, []);

  return (
    <div className="space-y-4">
      <h3 className="text-lg font-semibold">Device Pairing</h3>

      <Card className="p-6">
        <div className="text-center space-y-4">
          <p className="text-sm text-muted-foreground">
            Enter this code on your new device
          </p>
          <div className="text-4xl font-mono font-bold tracking-widest">
            {code || "------"}
          </div>
          <Button
            variant="outline"
            size="sm"
            onClick={fetchCode}
            disabled={loading}
          >
            <RefreshCw className={`h-4 w-4 mr-2 ${loading ? "animate-spin" : ""}`} />
            New Code
          </Button>
        </div>
      </Card>

      {pending.length > 0 && (
        <div className="space-y-2">
          <h4 className="text-sm font-medium">Pending Requests</h4>
          {pending.map((p) => (
            <Card key={p.fingerprint} className="p-4">
              <div className="flex items-center justify-between">
                <div>
                  <div className="font-medium">{p.deviceName}</div>
                  <Badge variant="outline" className="text-xs">
                    Waiting for approval
                  </Badge>
                </div>
                <div className="flex gap-2">
                  <Button
                    size="icon"
                    variant="outline"
                    onClick={() => handleApprove(p.fingerprint)}
                  >
                    <Check className="h-4 w-4 text-green-600" />
                  </Button>
                  <Button
                    size="icon"
                    variant="outline"
                    onClick={() => handleReject(p.fingerprint)}
                  >
                    <X className="h-4 w-4 text-red-600" />
                  </Button>
                </div>
              </div>
            </Card>
          ))}
        </div>
      )}
    </div>
  );
}
```

**Step 5: Update Settings page**

Modify `src/app/settings/page.tsx`:

```tsx
"use client";

import { useState } from "react";
import { Header } from "@/components/layout/header";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { WorkspaceList } from "@/components/settings/workspace-list";
import { AddWorkspaceDialog } from "@/components/settings/add-workspace-dialog";
import { DeviceList } from "@/components/settings/device-list";
import { PairingDisplay } from "@/components/settings/pairing-display";

export default function SettingsPage() {
  const [dialogOpen, setDialogOpen] = useState(false);
  const [refreshKey, setRefreshKey] = useState(0);

  return (
    <div className="flex flex-col h-full">
      <Header
        title="Settings"
        description="Configure workspaces and devices"
      />
      <div className="flex-1 p-6">
        <Tabs defaultValue="workspaces" className="space-y-6">
          <TabsList>
            <TabsTrigger value="workspaces">Workspaces</TabsTrigger>
            <TabsTrigger value="devices">Devices</TabsTrigger>
          </TabsList>

          <TabsContent value="workspaces" className="space-y-6">
            <WorkspaceList
              key={refreshKey}
              onAddClick={() => setDialogOpen(true)}
            />
          </TabsContent>

          <TabsContent value="devices" className="space-y-6">
            <PairingDisplay />
            <DeviceList />
          </TabsContent>
        </Tabs>
      </div>

      <AddWorkspaceDialog
        open={dialogOpen}
        onOpenChange={setDialogOpen}
        onSuccess={() => setRefreshKey((k) => k + 1)}
      />
    </div>
  );
}
```

**Step 6: Create devices API routes**

Create `src/app/api/auth/devices/route.ts`:

```typescript
import { NextResponse } from "next/server";
import { db, authorizedDevices } from "@/lib/db";
import { eq } from "drizzle-orm";

export async function GET() {
  try {
    const devices = await db
      .select()
      .from(authorizedDevices)
      .where(eq(authorizedDevices.isRevoked, false));
    return NextResponse.json(devices);
  } catch (error) {
    console.error("Failed to fetch devices:", error);
    return NextResponse.json(
      { error: "Failed to fetch devices" },
      { status: 500 }
    );
  }
}
```

Create `src/app/api/auth/devices/[id]/route.ts`:

```typescript
import { NextRequest, NextResponse } from "next/server";
import { db, authorizedDevices } from "@/lib/db";
import { eq } from "drizzle-orm";

export async function DELETE(
  request: NextRequest,
  { params }: { params: Promise<{ id: string }> }
) {
  try {
    const { id } = await params;
    await db
      .update(authorizedDevices)
      .set({ isRevoked: true })
      .where(eq(authorizedDevices.id, parseInt(id)));

    return NextResponse.json({ success: true });
  } catch (error) {
    console.error("Failed to revoke device:", error);
    return NextResponse.json(
      { error: "Failed to revoke device" },
      { status: 500 }
    );
  }
}
```

**Step 7: Commit**

```bash
cd /Users/gregoryerrl/Projects/bot-hq
git add -A
git commit -m "feat: add settings page with workspace and device management"
```

---

## Task 9: Final Verification

**Step 1: Run the app and verify all pages**

Run:
```bash
cd /Users/gregoryerrl/Projects/bot-hq && npm run build
```

Expected: Build succeeds with no errors

**Step 2: Run dev server and test**

Run:
```bash
cd /Users/gregoryerrl/Projects/bot-hq && npm run dev
```

Then manually verify:
- http://localhost:3000 - Taskboard loads
- http://localhost:3000/pending - Pending page loads
- http://localhost:3000/logs - Logs page loads
- http://localhost:3000/settings - Settings with tabs loads

**Step 3: Create workspace via API test**

```bash
curl -X POST http://localhost:3000/api/workspaces \
  -H "Content-Type: application/json" \
  -d '{"name":"test-project","repoPath":"~/Projects/test"}'
```

Expected: Workspace created with id returned

**Step 4: Final commit if any fixes needed**

```bash
cd /Users/gregoryerrl/Projects/bot-hq
git status
# If changes exist:
git add -A
git commit -m "fix: address build/runtime issues"
```

---

## Summary

Phase 1 Foundation creates:
- Next.js 14+ project with TypeScript
- SQLite database with Drizzle ORM (all tables from design)
- shadcn/ui components
- App layout with sidebar navigation
- Taskboard, Pending, Logs, Settings pages
- Workspace CRUD API
- Device authorization system with pairing codes
- Settings UI for managing workspaces and devices

Total commits: ~9 focused commits
