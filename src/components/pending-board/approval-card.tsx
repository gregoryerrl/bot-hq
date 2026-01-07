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
    <Card className="p-3 md:p-4">
      <div className="flex flex-col gap-3">
        <div className="flex flex-wrap items-center gap-2">
          <Badge variant="secondary" className="text-xs">
            {typeLabels[approval.type]}
          </Badge>
          {approval.workspaceName && (
            <Badge variant="outline" className="text-xs">
              {approval.workspaceName}
            </Badge>
          )}
          {approval.githubIssueNumber && (
            <span className="text-xs text-muted-foreground">
              Issue #{approval.githubIssueNumber}
            </span>
          )}
        </div>

        {approval.taskTitle && (
          <h3 className="font-medium text-sm md:text-base">
            {approval.taskTitle}
          </h3>
        )}

        <div className="flex items-center gap-2 p-2 bg-muted rounded text-xs md:text-sm font-mono overflow-x-auto">
          <Terminal className="h-4 w-4 text-muted-foreground flex-shrink-0" />
          <code className="whitespace-nowrap">{approval.command}</code>
        </div>

        {approval.reason && (
          <p className="text-xs md:text-sm text-muted-foreground">
            {approval.reason}
          </p>
        )}

        <div className="flex items-center justify-end gap-2">
          <Button
            size="sm"
            variant="outline"
            className="text-green-600 hover:text-green-700 hover:bg-green-50"
            onClick={() => onApprove(approval.id)}
          >
            <Check className="h-4 w-4 mr-1" />
            <span className="hidden sm:inline">Approve</span>
          </Button>
          <Button
            size="sm"
            variant="outline"
            className="text-red-600 hover:text-red-700 hover:bg-red-50"
            onClick={() => onReject(approval.id)}
          >
            <X className="h-4 w-4 mr-1" />
            <span className="hidden sm:inline">Reject</span>
          </Button>
        </div>
      </div>
    </Card>
  );
}
