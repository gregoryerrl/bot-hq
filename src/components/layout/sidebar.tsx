"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";
import { cn } from "@/lib/utils";
import {
  LayoutDashboard,
  Clock,
  ScrollText,
  Settings,
  Globe,
  MessageSquare,
  Terminal,
  FileText,
  Puzzle,
  Box,
  LucideIcon,
} from "lucide-react";
import * as LucideIcons from "lucide-react";
import { usePluginUI } from "@/hooks/use-plugin-ui";

const navItems = [
  { href: "/", label: "Taskboard", icon: LayoutDashboard },
  { href: "/pending", label: "Pending", icon: Clock },
  { href: "/chat", label: "Claude Chat", icon: MessageSquare },
  { href: "/terminal", label: "Terminal", icon: Terminal },
  { href: "/docs", label: "Docs", icon: FileText },
  { href: "/logs", label: "Logs", icon: ScrollText },
  { href: "/plugins", label: "Plugins", icon: Puzzle },
  { href: "/settings", label: "Settings", icon: Settings },
  { href: "/claude-settings", label: "Claude Settings", icon: Globe },
];

function getIconComponent(iconName: string): LucideIcon {
  // Convert kebab-case to PascalCase
  const pascalCase = iconName
    .split("-")
    .map(word => word.charAt(0).toUpperCase() + word.slice(1))
    .join("");

  const icons = LucideIcons as unknown as Record<string, LucideIcon>;
  return icons[pascalCase] || Box;
}

export function Sidebar() {
  const pathname = usePathname();
  const { tabs: pluginTabs, loading } = usePluginUI();

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

        {/* Plugin Tabs Section */}
        {!loading && pluginTabs.length > 0 && (
          <>
            <div className="pt-4 pb-2">
              <span className="px-3 text-xs font-semibold text-muted-foreground uppercase tracking-wider">
                Plugins
              </span>
            </div>
            {pluginTabs.map((tab) => {
              const Icon = getIconComponent(tab.icon);
              const href = `/plugins/${tab.pluginName}/${tab.id}`;
              const isActive = pathname === href;

              return (
                <Link
                  key={`${tab.pluginName}-${tab.id}`}
                  href={href}
                  className={cn(
                    "flex items-center gap-3 px-3 py-2 rounded-md text-sm transition-colors",
                    isActive
                      ? "bg-primary text-primary-foreground"
                      : "hover:bg-muted"
                  )}
                >
                  <Icon className="h-4 w-4" />
                  {tab.label}
                </Link>
              );
            })}
          </>
        )}
      </nav>
    </aside>
  );
}
