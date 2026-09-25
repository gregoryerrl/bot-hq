import { useEffect } from "react";
import { NavLink, Outlet, useNavigate } from "react-router-dom";
import { cn } from "../lib/cn";
import { PendingTray } from "../components/PendingTray";
import { UpdateBanner } from "../components/UpdateBanner";
import { DiagnosticsAskCard } from "../components/DiagnosticsAskCard";
import { useHealthStore, appHealthSummary } from "../stores/health";
import { useActivityStore, anyBusy } from "../stores/activity";
import { useTauriQuery } from "../hooks/useInvoke";
import type { BuildInfo, InstalledPluginView } from "../lib/bindings";

// Topbar tabs for enabled plugins that contribute a panel (manifest
// `slots[].panel_route`). The `plugin:*` events invalidate this query from
// `Providers.tsx`'s GlobalEventSync (round 8) — the same query key
// PluginManager uses, one cache, one set of listeners.
function PluginNavTabs() {
  const list = useTauriQuery<InstalledPluginView[]>("list_installed_plugins", {});

  const withPanel = (list.data ?? []).filter(
    (p) => p.enabled && p.manifest.slots?.some((s) => s.panel_route),
  );
  return (
    <>
      {withPanel.map((p) => (
        <NavLink key={p.id} to={`/plugins/view/${p.id}`} className={navLinkClass}>
          {p.name}
        </NavLink>
      ))}
    </>
  );
}

// B3: app-wide agent-health status in the footer (replaces the hardcoded green
// "Online"). Worst-of all sessions from the B2 health store — green when all OK,
// amber while any agent is recovering, red when any has stopped, grey when no
// session is live. Liveness comes from the activity store (one entry per live
// session, seeded on mount, cleared on close): the health map is transition-
// only, so on its own it read "idle" over two working agents (round 11).
function FooterStatus() {
  const bySession = useHealthStore((s) => s.bySession);
  const liveSessions = useActivityStore((s) => Object.keys(s.bySession).length);
  const { state, count } = appHealthSummary(bySession, liveSessions);
  const cfg = {
    ok: { dot: "bg-success", label: "Agents: OK" },
    retrying: { dot: "bg-warning animate-pulse", label: `${count} recovering` },
    stalled: { dot: "bg-error animate-pulse", label: `${count} stalled` },
    dead: { dot: "bg-error", label: `${count} stopped` },
    idle: { dot: "bg-outline-variant", label: "No live sessions" },
  }[state];
  return (
    <span
      className="flex cursor-default items-center gap-1 font-code-sm text-code-sm text-on-surface-variant"
      title={
        state === "ok"
          ? "All agents running"
          : state === "idle"
            ? "No session is live"
            : `${count} session${count === 1 ? "" : "s"} with ${state === "dead" ? "a stopped" : state === "stalled" ? "a stalled" : "a recovering"} agent`
      }
    >
      <span className={cn("size-2 rounded-full", cfg.dot)} />
      {cfg.label}
    </span>
  );
}

// The footer's left side (the user's ask, 2026-09-25): which build is running
// — version, the commit it was built from, and whether its program file
// changed since launch. The git hooks of every project exec the program file
// the app was LAUNCHED from, so a rebuild without a relaunch leaves the hooks
// on newer code than the app; the chip says so. Detail lives in the tooltip.
export function BuildStamp() {
  const { data } = useTauriQuery<BuildInfo | null>("app_build_info", {}, {
    refetchInterval: 60_000,
    refetchOnWindowFocus: true,
  });
  const build = data ?? null;
  const tag = build?.commit ?? (build?.profile === "debug" ? "dev" : null);
  const detail = build
    ? [
        `Build: ${build.profile}${build.commit ? ` · ${build.commit}` : ""}`,
        build.exe_path && `Program: ${build.exe_path}`,
        build.exe_built_at && `Built: ${build.exe_built_at}`,
        `Data: ${build.data_dir}`,
        build.schema_version != null && `Database: migration ${build.schema_version}`,
      ]
        .filter(Boolean)
        .join("\n")
    : undefined;
  return (
    <div className="flex min-w-0 items-center gap-2">
      <span
        className="min-w-0 cursor-default truncate font-label-caps text-label-caps text-primary"
        title={detail}
        data-testid="build-stamp"
      >
        bot-hq{build ? ` v${build.version}` : ""}
        {tag && <span className="text-on-surface-variant"> · {tag}</span>}
      </span>
      {build?.exe_state === "changed" && (
        <span
          className="shrink-0 cursor-default rounded bg-warning/15 px-1.5 py-0.5 font-code-sm text-code-sm text-warning"
          title="The program file changed since this app launched. Git hooks already run the new one; restart bot-hq to match it."
          data-testid="restart-pending"
        >
          restart pending
        </span>
      )}
      {build?.exe_state === "missing" && (
        <span
          className="shrink-0 cursor-default rounded bg-error/15 px-1.5 py-0.5 font-code-sm text-code-sm text-error"
          title="The program file is gone (a cargo clean?). Git hooks cannot run it, so every commit fails until it is rebuilt. Rebuild, then restart."
          data-testid="rebuild-needed"
        >
          rebuild needed
        </span>
      )}
    </div>
  );
}

// How many live sessions have a turn in flight — the relaunch-safety signal: a
// relaunch now would cut those turns off. Busy, cancelling, or any participant
// still busy counts, so a paused session whose last tool is still finishing is
// counted too (EYES P8). Hidden when no session is live (FooterStatus says so).
export function WorkingSessions() {
  const bySession = useActivityStore((s) => s.bySession);
  const busyBySession = useActivityStore((s) => s.busyBySession);
  const live = Object.keys(bySession);
  if (live.length === 0) return null;
  const working = live.filter(
    (id) =>
      bySession[id] === "busy" ||
      bySession[id] === "cancelling" ||
      anyBusy(busyBySession[id]),
  ).length;
  return (
    <span
      className="shrink-0 cursor-default font-code-sm text-code-sm text-on-surface-variant"
      title={
        working === 0
          ? "No session has a turn in flight — a relaunch now interrupts nothing"
          : `${working} session${working === 1 ? " has a turn" : "s have turns"} in flight — a relaunch now would cut ${working === 1 ? "it" : "them"} off`
      }
      data-testid="working-sessions"
    >
      {working} working
    </span>
  );
}

export function Shell() {
  const navigate = useNavigate();

  // App-wide shortcuts: ⌘/Ctrl-N opens the New-session dialog (the `?new=1`
  // param is consumed by Dashboard), ⌘/Ctrl-, opens Settings (the macOS
  // preferences convention). preventDefault keeps the webview from acting on
  // the browser meaning of the chord.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!(e.metaKey || e.ctrlKey)) return;
      if (e.key === "n") {
        e.preventDefault();
        navigate("/?new=1");
      } else if (e.key === ",") {
        e.preventDefault();
        navigate("/settings");
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [navigate]);

  return (
    <div className="flex h-screen flex-col bg-background font-body-md text-on-background">
      <header className="flex h-12 flex-shrink-0 items-center justify-between border-b border-outline-variant bg-surface px-grid-margin">
        <div className="flex h-full items-center gap-6">
          <h1 className="text-headline-lg font-headline-lg text-primary">
            bot-hq
          </h1>
          <nav className="flex h-full items-center gap-4">
            <NavLink to="/" end className={navLinkClass}>
              Dashboard
            </NavLink>
            <NavLink to="/cl" className={navLinkClass}>
              Context Library
            </NavLink>
            <NavLink to="/plugins" className={navLinkClass}>
              Plugins
            </NavLink>
            <PluginNavTabs />
            <NavLink to="/settings" className={navLinkClass}>
              Settings
            </NavLink>
          </nav>
        </div>
        <div className="flex items-center gap-4">
          <PendingTray />
        </div>
      </header>
      <UpdateBanner />
      <DiagnosticsAskCard />
      <main className="flex-1 overflow-hidden">
        <Outlet />
      </main>
      <footer
        className={cn(
          "flex h-10 flex-shrink-0 items-center justify-between gap-4",
          "border-t border-outline-variant bg-surface-container-lowest px-4",
        )}
      >
        {/* Fixed height: the left side truncates rather than wrapping or
            scrolling sideways (EYES F4). */}
        <BuildStamp />
        <div className="flex shrink-0 items-center gap-4">
          <WorkingSessions />
          <FooterStatus />
        </div>
      </footer>
    </div>
  );
}

const navLinkClass = ({ isActive }: { isActive: boolean }) =>
  cn(
    "flex h-full items-center px-2 font-headline-md text-headline-md transition-colors",
    isActive
      ? "border-b-2 border-primary text-primary"
      : "text-on-surface-variant hover:bg-surface-variant/50",
  );
