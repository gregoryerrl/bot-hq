import { NavLink, Outlet } from "react-router-dom";
import { cn } from "../lib/cn";
import { useEmmaStore } from "../stores/emma";
import { Button } from "../components/ui/Button";
import { EmmaOverlay } from "../components/EmmaOverlay";

const navItem = ({ isActive }: { isActive: boolean }) =>
  cn(
    "rounded px-3 py-1.5 text-sm font-medium",
    isActive
      ? "bg-neutral-800 text-neutral-100"
      : "text-neutral-400 hover:text-neutral-100 hover:bg-neutral-900",
  );

export function Shell() {
  const toggleEmma = useEmmaStore((s) => s.toggle);

  return (
    <div className="flex h-screen flex-col bg-neutral-950 text-neutral-100">
      <header className="flex items-center justify-between border-b border-neutral-800 px-4 py-2">
        <div className="flex items-center gap-1">
          <NavLink to="/" end className={navItem}>
            Dashboard
          </NavLink>
          <NavLink to="/cl" className={navItem}>
            Context Library
          </NavLink>
          <NavLink to="/plugins" className={navItem}>
            Plugins
          </NavLink>
          <NavLink to="/settings" className={navItem}>
            Settings
          </NavLink>
        </div>
        <Button variant="secondary" size="sm" onClick={() => toggleEmma()}>
          Emma
        </Button>
      </header>
      <main className="flex-1 overflow-hidden">
        <Outlet />
      </main>
      <EmmaOverlay />
    </div>
  );
}
