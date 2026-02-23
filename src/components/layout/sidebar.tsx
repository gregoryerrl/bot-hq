"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";
import { cn } from "@/lib/utils";
import {
  LayoutDashboard,
  ListTodo,
  Clock,
  ScrollText,
  Settings,
  Bot,
  FileText,
  FolderGit2,
  GitBranch,
} from "lucide-react";
import { useCallback } from "react";

const navItems = [
  { href: "/", label: "Dashboard", icon: LayoutDashboard },
  { href: "/taskboard", label: "Taskboard", icon: ListTodo },
  { href: "/pending", label: "Review", icon: Clock },
  { href: "/workspaces", label: "Workspaces", icon: FolderGit2 },
  { href: "/git", label: "Git", icon: GitBranch },
  { href: "/claude", label: "Claude", icon: Bot },
  { href: "/docs", label: "Docs", icon: FileText },
  { href: "/logs", label: "Logs", icon: ScrollText },
  { href: "/settings", label: "Settings", icon: Settings },
];

export function Sidebar() {
  const pathname = usePathname();

  // Force hard navigation to work around SSE connection blocking client-side navigation
  const handleNavigation = useCallback((e: React.MouseEvent<HTMLAnchorElement>, href: string) => {
    e.preventDefault();
    e.stopPropagation();
    // Use window.location.assign for reliable navigation when SSE connections are active
    // This forces a full page reload, properly cleaning up SSE connections
    if (typeof window !== 'undefined') {
      window.location.assign(href);
    }
  }, []);

  return (
    <aside className="hidden md:flex w-56 flex-col border-r bg-muted/30">
      <div className="p-4 border-b">
        <h2 className="text-lg font-semibold">Bot-HQ</h2>
      </div>
      <nav className="flex-1 p-4 space-y-2 overflow-y-auto">
        {navItems.map((item) => {
          const Icon = item.icon;
          const isActive = pathname === item.href;

          return (
            <Link
              key={item.href}
              href={item.href}
              onClick={(e) => handleNavigation(e, item.href)}
              className={cn(
                "flex items-center gap-3 px-3 py-2 rounded-md text-sm transition-colors",
                isActive
                  ? "bg-primary text-primary-foreground"
                  : "hover:bg-muted"
              )}
            >
              <Icon className="h-4 w-4" />
              {item.label}
            </Link>
          );
        })}
      </nav>
    </aside>
  );
}
