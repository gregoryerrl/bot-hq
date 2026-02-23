"use client";

import { useState, useCallback } from "react";
import Link from "next/link";
import { usePathname } from "next/navigation";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import {
  Menu,
  X,
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

const navItems = [
  { href: "/", label: "Dashboard", icon: LayoutDashboard },
  { href: "/taskboard", label: "Taskboard", icon: ListTodo },
  { href: "/pending", label: "Pending", icon: Clock },
  { href: "/workspaces", label: "Workspaces", icon: FolderGit2 },
  { href: "/git", label: "Git", icon: GitBranch },
  { href: "/claude", label: "Claude", icon: Bot },
  { href: "/docs", label: "Docs", icon: FileText },
  { href: "/logs", label: "Logs", icon: ScrollText },
  { href: "/settings", label: "Settings", icon: Settings },
];

export function MobileNav() {
  const [isOpen, setIsOpen] = useState(false);
  const pathname = usePathname();

  // Force hard navigation to work around SSE connection blocking client-side navigation
  const handleNavigation = useCallback((e: React.MouseEvent<HTMLAnchorElement>, href: string) => {
    e.preventDefault();
    setIsOpen(false);
    // Use window.location for reliable navigation when SSE connections are active
    window.location.href = href;
  }, []);

  return (
    <>
      {/* Mobile Menu Button - Only visible on small screens */}
      <div className="fixed top-4 left-4 z-50 md:hidden">
        <Button
          variant="outline"
          size="icon"
          onClick={() => setIsOpen(!isOpen)}
        >
          {isOpen ? <X className="h-4 w-4" /> : <Menu className="h-4 w-4" />}
        </Button>
      </div>

      {/* Mobile Navigation Overlay */}
      {isOpen && (
        <div
          className="fixed inset-0 bg-black/50 z-40 md:hidden"
          onClick={() => setIsOpen(false)}
        />
      )}

      {/* Mobile Navigation Panel */}
      <div
        className={cn(
          "fixed left-0 top-0 h-full w-64 bg-background border-r z-50 transform transition-transform duration-200 md:hidden",
          isOpen ? "translate-x-0" : "-translate-x-full"
        )}
      >
        <div className="p-4 border-b mt-14">
          <h2 className="text-lg font-semibold">Bot-HQ</h2>
        </div>
        <nav className="p-4 space-y-2">
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
      </div>
    </>
  );
}
