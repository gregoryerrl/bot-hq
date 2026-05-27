import { useState } from "react";
import { NavLink, Outlet } from "react-router-dom";
import { cn } from "../lib/cn";
import { useEmmaStore } from "../stores/emma";
import { EmmaOverlay } from "../components/EmmaOverlay";
import { PendingTray } from "../components/PendingTray";

export function Shell() {
  const toggleEmma = useEmmaStore((s) => s.toggle);
  const emmaOpen = useEmmaStore((s) => s.open);
  const [search, setSearch] = useState("");

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
          <div className="relative hidden sm:block">
            <SearchIcon className="pointer-events-none absolute left-2 top-1/2 -translate-y-1/2 text-on-surface-variant" />
            <input
              type="text"
              value={search}
              onChange={(e) => setSearch(e.target.value)}
              placeholder="Search sessions, agents, tasks…"
              className={cn(
                "w-64 rounded border border-outline-variant bg-surface-container",
                "py-1 pl-8 pr-3 font-code-sm text-code-sm text-on-surface",
                "placeholder:text-on-surface-variant",
                "focus:border-primary focus:outline-none focus:ring-1 focus:ring-primary",
              )}
            />
          </div>
          <PendingTray />
          <div className="ml-2 flex items-center border-l border-outline-variant pl-4">
            <button
              onClick={() => toggleEmma()}
              aria-pressed={emmaOpen}
              aria-label={emmaOpen ? "Close Emma chat" : "Open Emma chat"}
              className={cn(
                "flex items-center gap-2 rounded border border-primary bg-primary-container",
                "px-3 py-1 font-code-sm text-code-sm text-on-primary-container",
                "shadow-inner ring-1 ring-primary/50 transition-colors",
              )}
            >
              <PersonIcon />
              Emma
            </button>
          </div>
        </div>
      </header>
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
          <span className="flex cursor-default items-center gap-1 font-code-sm text-code-sm text-on-surface-variant">
            <span className="size-2 rounded-full bg-emerald-500" />
            Status: Online
          </span>
          <a
            href="#"
            className="hidden font-code-sm text-code-sm text-on-surface-variant hover:text-primary sm:block"
          >
            API Docs
          </a>
          <a
            href="#"
            className="hidden font-code-sm text-code-sm text-on-surface-variant hover:text-primary sm:block"
          >
            Support
          </a>
        </div>
      </footer>
      <EmmaOverlay />
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

function SearchIcon({ className }: { className?: string }) {
  return (
    <svg
      className={cn("size-[18px]", className)}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={2}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
    >
      <circle cx={11} cy={11} r={7} />
      <path d="M21 21l-4.35-4.35" />
    </svg>
  );
}

function PersonIcon({ className }: { className?: string }) {
  return (
    <svg
      className={cn("size-4", className)}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={2}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
    >
      <circle cx={12} cy={8} r={4} />
      <path d="M20 21a8 8 0 00-16 0" />
    </svg>
  );
}
