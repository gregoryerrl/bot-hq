"use client";

import { useState } from "react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { Badge } from "@/components/ui/badge";
import { Check, Trash2, RotateCcw, GitBranch, FileCode } from "lucide-react";
import { Task } from "@/lib/db/schema";

interface DiffFile {
  filename: string;
  additions: number;
  deletions: number;
}

interface DiffReviewCardProps {
  task: Task & { workspaceName?: string };
  diff: {
    branch: string;
    baseBranch: string;
    files: DiffFile[];
    totalAdditions: number;
    totalDeletions: number;
  };
  onAccept: (taskId: number) => void;
  onDelete: (taskId: number) => void;
  onRetry: (taskId: number, feedback: string) => void;
}

export function DiffReviewCard({
  task,
  diff,
  onAccept,
  onDelete,
  onRetry,
}: DiffReviewCardProps) {
  const [feedback, setFeedback] = useState("");
  const [showFeedback, setShowFeedback] = useState(false);

  const handleRetry = () => {
    if (feedback.trim()) {
      onRetry(task.id, feedback);
      setFeedback("");
      setShowFeedback(false);
    }
  };

  return (
    <Card>
      <CardHeader>
        <div className="flex items-center justify-between">
          <div>
            <CardTitle className="flex items-center gap-2">
              <span>#{task.id}</span>
              <span>{task.title}</span>
            </CardTitle>
            <div className="flex items-center gap-2 mt-2 text-sm text-muted-foreground">
              <GitBranch className="h-4 w-4" />
              <code>{diff.branch}</code>
              <span>→</span>
              <code>{diff.baseBranch}</code>
            </div>
          </div>
          {task.workspaceName && (
            <Badge variant="outline">{task.workspaceName}</Badge>
          )}
        </div>
      </CardHeader>
      <CardContent className="space-y-4">
        {/* File changes */}
        <div className="border rounded-lg p-3 space-y-2">
          <div className="flex items-center justify-between text-sm">
            <span className="font-medium">Changes</span>
            <div className="flex gap-2">
              <span className="text-green-600">+{diff.totalAdditions}</span>
              <span className="text-red-600">-{diff.totalDeletions}</span>
            </div>
          </div>
          <div className="space-y-1 max-h-48 overflow-y-auto">
            {diff.files.map((file, i) => (
              <div key={i} className="flex items-center justify-between text-xs">
                <div className="flex items-center gap-2 truncate">
                  <FileCode className="h-3 w-3 text-muted-foreground" />
                  <span className="truncate">{file.filename}</span>
                </div>
                <div className="flex gap-2 flex-shrink-0">
                  <span className="text-green-600">+{file.additions}</span>
                  <span className="text-red-600">-{file.deletions}</span>
                </div>
              </div>
            ))}
          </div>
        </div>

        {/* Feedback input */}
        {showFeedback && (
          <div className="space-y-2">
            <Textarea
              placeholder="What should be changed?"
              value={feedback}
              onChange={(e) => setFeedback(e.target.value)}
              className="min-h-[100px]"
            />
            <div className="flex gap-2">
              <Button size="sm" onClick={handleRetry} disabled={!feedback.trim()}>
                Send Feedback
              </Button>
              <Button
                size="sm"
                variant="outline"
                onClick={() => setShowFeedback(false)}
              >
                Cancel
              </Button>
            </div>
          </div>
        )}

        {/* Action buttons */}
        {!showFeedback && (
          <div className="flex gap-2">
            <Button
              className="flex-1"
              onClick={() => onAccept(task.id)}
            >
              <Check className="h-4 w-4 mr-2" />
              Accept
            </Button>
            <Button
              variant="outline"
              onClick={() => setShowFeedback(true)}
            >
              <RotateCcw className="h-4 w-4 mr-2" />
              Retry
            </Button>
            <Button
              variant="destructive"
              onClick={() => onDelete(task.id)}
            >
              <Trash2 className="h-4 w-4 mr-2" />
              Delete
            </Button>
          </div>
        )}
      </CardContent>
    </Card>
  );
}
