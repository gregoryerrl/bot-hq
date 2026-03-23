"use client";

import { Card, CardHeader, CardTitle, CardDescription, CardContent, CardFooter } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { formatDistanceToNow } from "date-fns";

interface ProjectCardProps {
  id: number;
  name: string;
  description?: string | null;
  status: string;
  taskCounts?: Record<string, number>;
  diagramCount?: number;
  updatedAt: string | Date;
  onClick: (id: number) => void;
}

const stateColors: Record<string, string> = {
  todo: "",
  in_progress: "border-orange-500 text-orange-600",
  done: "border-green-500 text-green-600",
  blocked: "border-red-500 text-red-600",
};

const stateLabels: Record<string, string> = {
  todo: "Todo",
  in_progress: "In Progress",
  done: "Done",
  blocked: "Blocked",
};

export function ProjectCard({
  id,
  name,
  description,
  status,
  taskCounts,
  diagramCount,
  updatedAt,
  onClick,
}: ProjectCardProps) {
  const hasTaskCounts = taskCounts && Object.keys(taskCounts).length > 0;

  return (
    <Card
      className="cursor-pointer transition-colors hover:bg-muted/50"
      onClick={() => onClick(id)}
    >
      <CardHeader>
        <div className="flex items-center justify-between">
          <CardTitle className="truncate font-medium text-base">{name}</CardTitle>
          <Badge variant={status === "archived" ? "secondary" : "default"} className="ml-2 shrink-0">
            {status}
          </Badge>
        </div>
        {description && (
          <CardDescription className="line-clamp-2">{description}</CardDescription>
        )}
      </CardHeader>

      {hasTaskCounts && (
        <CardContent>
          <div className="flex flex-wrap gap-1.5">
            {Object.entries(taskCounts).map(([state, count]) => (
              <Badge
                key={state}
                variant={state === "todo" ? "default" : "outline"}
                className={stateColors[state] || ""}
              >
                {stateLabels[state] || state}: {count}
              </Badge>
            ))}
          </div>
        </CardContent>
      )}

      <CardFooter className="text-xs text-muted-foreground justify-between">
        {diagramCount !== undefined && diagramCount > 0 && (
          <span>{diagramCount} {diagramCount === 1 ? "diagram" : "diagrams"}</span>
        )}
        <span className="ml-auto">
          Updated {formatDistanceToNow(new Date(updatedAt), { addSuffix: true })}
        </span>
      </CardFooter>
    </Card>
  );
}
