"use client";

import { useState } from "react";
import Link from "next/link";
import { usePathname } from "next/navigation";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import { Menu, X, LayoutDashboard, Clock, ScrollText, Settings, Globe } from "lucide-react";

const navItems = [
  { href: "/", label: "Taskboard", icon: LayoutDashboard },
  { href: "/pending", label: "Pending", icon: Clock },
  { href: "/logs", label: "Logs", icon: ScrollText },
  { href: "/settings", label: "Settings", icon: Settings },
  { href: "/claude-settings", label: "Claude Settings", icon: Globe },
];

export function MobileNav() {
  const [isOpen, setIsOpen] = useState(false);
  const pathname = usePathname();

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
                onClick={() => setIsOpen(false)}
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
