"use client";

import { Card } from "@/components/ui/card";
import { LucideIcon } from "lucide-react";

interface StatCardProps {
  title: string;
  count: number;
  icon: LucideIcon;
  color: string;
  href?: string;
}

export function StatCard({ title, count, icon: Icon, color, href }: StatCardProps) {
  const content = (
    <Card className={`p-4 ${href ? "cursor-pointer hover:bg-muted/50 transition-colors" : ""}`}>
      <div className="flex items-center justify-between">
        <div>
          <p className="text-sm text-muted-foreground">{title}</p>
          <p className="text-3xl font-bold mt-1">{count}</p>
        </div>
        <div className={`p-3 rounded-full ${color}`}>
          <Icon className="h-5 w-5 text-white" />
        </div>
      </div>
    </Card>
  );

  if (href) {
    return (
      <a
        href={href}
        onClick={(e) => {
          e.preventDefault();
          window.location.assign(href);
        }}
      >
        {content}
      </a>
    );
  }

  return content;
}
