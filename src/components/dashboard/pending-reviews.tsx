"use client";

import { Card } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Task } from "@/lib/db/schema";

interface PendingReviewsProps {
  tasks: Array<Task & { workspaceName?: string }>;
}

export function PendingReviews({ tasks }: PendingReviewsProps) {
  return (
    <Card className="p-4">
      <div className="flex items-center justify-between mb-3">
        <h3 className="text-sm font-medium">Pending Reviews</h3>
        <a
          href="/pending"
          onClick={(e) => {
            e.preventDefault();
            window.location.assign("/pending");
          }}
          className="text-xs text-muted-foreground hover:text-foreground"
        >
          Review all
        </a>
      </div>
      {tasks.length === 0 ? (
        <p className="text-sm text-muted-foreground py-4 text-center">
          No pending reviews
        </p>
      ) : (
        <div className="space-y-2">
          {tasks.map((task) => (
            <div
              key={task.id}
              className="flex items-center gap-2 py-2 border-b last:border-0"
            >
              <span className="text-sm truncate flex-1">{task.title}</span>
              {task.branchName && (
                <Badge variant="secondary" className="text-xs shrink-0 font-mono">
                  {task.branchName}
                </Badge>
              )}
              {task.workspaceName && (
                <Badge variant="outline" className="text-xs shrink-0">
                  {task.workspaceName}
                </Badge>
              )}
            </div>
          ))}
        </div>
      )}
    </Card>
  );
}
