import { NavLink, Outlet } from "react-router-dom";
import { cn } from "../lib/cn";
import { useEmmaStore } from "../stores/emma";
import { EmmaOverlay } from "../components/EmmaOverlay";
import { PendingTray } from "../components/PendingTray";

const navItem = ({ isActive }: { isActive: boolean }) =>
  cn(
    "relative inline-flex h-9 items-center rounded-md px-3 text-sm font-medium transition-colors",
    isActive
      ? "text-neutral-100"
      : "text-neutral-400 hover:text-neutral-100",
  );

const navUnderline = ({ isActive }: { isActive: boolean }) =>
  cn(
    "absolute inset-x-3 -bottom-2 h-px transition-colors",
    isActive ? "bg-accent" : "bg-transparent",
  );

function NavTab({ to, end, children }: { to: string; end?: boolean; children: React.ReactNode }) {
  return (
    <NavLink to={to} end={end} className={navItem}>
      {({ isActive }) => (
        <>
          <span>{children}</span>
          <span className={navUnderline({ isActive })} />
        </>
      )}
    </NavLink>
  );
}

export function Shell() {
  const toggleEmma = useEmmaStore((s) => s.toggle);
  const emmaOpen = useEmmaStore((s) => s.open);

  return (
    <div className="flex h-screen flex-col bg-canvas text-neutral-100">
      <header className="flex items-center justify-between border-b border-default bg-canvas px-4 py-2">
        <div className="flex items-center gap-3">
          <span className="select-none text-sm font-bold tracking-tight text-neutral-100">
            bot-hq
          </span>
          <span className="h-5 w-px bg-default" />
          <nav className="flex items-center gap-1">
            <NavTab to="/" end>Dashboard</NavTab>
            <NavTab to="/cl">Context Library</NavTab>
            <NavTab to="/plugins">Plugins</NavTab>
            <NavTab to="/settings">Settings</NavTab>
          </nav>
        </div>
        <div className="flex items-center gap-1">
          <PendingTray />
          <button
            onClick={() => toggleEmma()}
            aria-pressed={emmaOpen}
            aria-label={emmaOpen ? "Close Emma chat" : "Open Emma chat"}
            className={cn(
              "inline-flex h-8 items-center gap-1.5 rounded-md border px-2.5 text-xs font-medium transition-colors",
              emmaOpen
                ? "border-author-emma/60 bg-author-emma/10 text-author-emma"
                : "border-default text-neutral-300 hover:border-author-emma/40 hover:text-neutral-100",
            )}
          >
            <span
              className={cn(
                "size-1.5 rounded-full",
                emmaOpen ? "bg-author-emma" : "bg-author-emma/50",
              )}
            />
            Emma
          </button>
        </div>
      </header>
      <main className="flex-1 overflow-hidden">
        <Outlet />
      </main>
      <EmmaOverlay />
    </div>
  );
}
