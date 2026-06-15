import { useEffect } from "react";
import { NavLink, Outlet, useNavigate } from "react-router-dom";
import { cn } from "../lib/cn";
import { PendingTray } from "../components/PendingTray";
import { UpdateBanner } from "../components/UpdateBanner";
import { useHealthStore, appHealthSummary } from "../stores/health";

// B3: app-wide agent-health status in the footer (replaces the hardcoded green
// "Online"). Worst-of all sessions from the B2 health store — green when all OK,
// amber while any agent is recovering, red when any has stopped.
function FooterStatus() {
  const bySession = useHealthStore((s) => s.bySession);
  const { state, count } = appHealthSummary(bySession);
  const cfg = {
    ok: { dot: "bg-success", label: "Agents: OK" },
    retrying: { dot: "bg-warning animate-pulse", label: `${count} recovering` },
    dead: { dot: "bg-error", label: `${count} stopped` },
  }[state];
  return (
    <span
      className="flex cursor-default items-center gap-1 font-code-sm text-code-sm text-on-surface-variant"
      title={
        state === "ok"
          ? "All agents running"
          : `${count} session${count === 1 ? "" : "s"} with ${state === "dead" ? "a stopped" : "a recovering"} agent`
      }
    >
      <span className={cn("size-2 rounded-full", cfg.dot)} />
      {cfg.label}
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
          <nav className="flex h-full items-center gap-4 pt-1">
            <NavLink to="/" end className={navLinkClass}>
              Dashboard
            </NavLink>
            <NavLink to="/cl" className={navLinkClass}>
              Context Library
            </NavLink>
            <NavLink to="/plugins" className={navLinkClass}>
              Plugins
            </NavLink>
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
      <main className="flex-1 overflow-hidden">
        <Outlet />
      </main>
      <footer
        className={cn(
          "flex h-10 flex-shrink-0 items-center justify-between",
          "border-t border-outline-variant bg-surface-container-lowest px-4",
        )}
      >
        <span className="font-label-caps text-label-caps text-primary">
          &copy; {new Date().getFullYear()} BOT-HQ INDUSTRIAL ORCHESTRATION
        </span>
        <div className="flex items-center gap-4">
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
