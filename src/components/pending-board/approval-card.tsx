"use client";

import { Card } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Check, X, Terminal } from "lucide-react";
import { Approval } from "@/lib/db/schema";

interface ApprovalCardProps {
  approval: Approval & {
    taskTitle?: string;
    workspaceName?: string;
    githubIssueNumber?: number;
  };
  onApprove: (id: number) => void;
  onReject: (id: number) => void;
}

const typeLabels: Record<string, string> = {
  git_push: "Git Push",
  external_command: "External Command",
  deploy: "Deploy",
};

export function ApprovalCard({
  approval,
  onApprove,
  onReject,
}: ApprovalCardProps) {
  return (
    <Card className="p-4">
      <div className="flex items-start justify-between gap-4">
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-2 mb-2">
            <Badge variant="secondary">{typeLabels[approval.type]}</Badge>
            {approval.workspaceName && (
              <Badge variant="outline">{approval.workspaceName}</Badge>
            )}
            {approval.githubIssueNumber && (
              <span className="text-sm text-muted-foreground">
                Issue #{approval.githubIssueNumber}
              </span>
            )}
          </div>
          {approval.taskTitle && (
            <h3 className="font-medium mb-2">{approval.taskTitle}</h3>
          )}
          <div className="flex items-center gap-2 p-2 bg-muted rounded text-sm font-mono">
            <Terminal className="h-4 w-4 text-muted-foreground flex-shrink-0" />
            <code className="truncate">{approval.command}</code>
          </div>
          {approval.reason && (
            <p className="text-sm text-muted-foreground mt-2">
              {approval.reason}
            </p>
          )}
        </div>
        <div className="flex items-center gap-2">
          <Button
            size="sm"
            variant="outline"
            className="text-green-600 hover:text-green-700 hover:bg-green-50"
            onClick={() => onApprove(approval.id)}
          >
            <Check className="h-4 w-4" />
          </Button>
          <Button
            size="sm"
            variant="outline"
            className="text-red-600 hover:text-red-700 hover:bg-red-50"
            onClick={() => onReject(approval.id)}
          >
            <X className="h-4 w-4" />
          </Button>
        </div>
      </div>
    </Card>
  );
}
