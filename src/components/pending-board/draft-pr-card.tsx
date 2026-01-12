"use client";

import { useState } from "react";
import { Card } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogFooter,
} from "@/components/ui/dialog";
import {
  Check,
  X,
  MessageSquare,
  GitBranch,
  ChevronDown,
  ChevronRight,
  FileText,
  Plus,
  Minus,
} from "lucide-react";
import { Approval } from "@/lib/db/schema";
import { PluginActionCheckboxes } from "@/components/plugins/plugin-action-checkboxes";

interface DiffFile {
  path: string;
  additions: number;
  deletions: number;
}

interface DiffSummary {
  files: DiffFile[];
  totalAdditions: number;
  totalDeletions: number;
}

interface DraftPRCardProps {
  approval: Approval & {
    taskTitle?: string;
    workspaceName?: string;
    githubIssueNumber?: number;
  };
  onApprove: (id: number, docRequest?: string, pluginActions?: string[]) => void;
  onReject: (id: number) => void;
  onRequestChanges: (id: number, instructions: string) => void;
}

export function DraftPRCard({
  approval,
  onApprove,
  onReject,
  onRequestChanges,
}: DraftPRCardProps) {
  const [showFiles, setShowFiles] = useState(false);
  const [showRequestChanges, setShowRequestChanges] = useState(false);
  const [showApproveDialog, setShowApproveDialog] = useState(false);
  const [instructions, setInstructions] = useState("");
  const [docRequest, setDocRequest] = useState("");
  const [loading, setLoading] = useState(false);
  const [selectedPluginActions, setSelectedPluginActions] = useState<string[]>([]);

  const commitMessages: string[] = approval.commitMessages
    ? JSON.parse(approval.commitMessages)
    : [];

  const diffSummary: DiffSummary = approval.diffSummary
    ? JSON.parse(approval.diffSummary)
    : { files: [], totalAdditions: 0, totalDeletions: 0 };

  const handleRequestChanges = async () => {
    if (!instructions.trim()) return;
    setLoading(true);
    await onRequestChanges(approval.id, instructions);
    setShowRequestChanges(false);
    setInstructions("");
    setLoading(false);
  };

  const handleApprove = async () => {
    setLoading(true);
    await onApprove(approval.id, docRequest.trim() || undefined, selectedPluginActions);
    setShowApproveDialog(false);
    setDocRequest("");
    setSelectedPluginActions([]);
    setLoading(false);
  };

  return (
    <>
      <Card className="p-4">
        <div className="flex flex-col gap-3">
          {/* Header */}
          <div className="flex flex-wrap items-center gap-2">
            <Badge variant="default" className="text-xs">
              Draft PR
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

          {/* Title */}
          {approval.taskTitle && (
            <h3 className="font-medium">{approval.taskTitle}</h3>
          )}

          {/* Branch */}
          <div className="flex items-center gap-2 text-sm text-muted-foreground">
            <GitBranch className="h-4 w-4" />
            <code className="bg-muted px-2 py-0.5 rounded text-xs">
              {approval.branchName}
            </code>
            <span>→</span>
            <code className="bg-muted px-2 py-0.5 rounded text-xs">
              {approval.baseBranch}
            </code>
          </div>

          {/* Commits */}
          {commitMessages.length > 0 && (
            <div className="space-y-1">
              <p className="text-sm font-medium">
                Commits ({commitMessages.length}):
              </p>
              <ul className="text-sm text-muted-foreground space-y-1 pl-4">
                {commitMessages.map((msg, i) => (
                  <li key={i} className="list-disc">
                    {msg}
                  </li>
                ))}
              </ul>
            </div>
          )}

          {/* Files changed */}
          <div className="border rounded-lg overflow-hidden">
            <button
              onClick={() => setShowFiles(!showFiles)}
              className="w-full flex items-center justify-between p-3 hover:bg-muted/50 transition-colors"
            >
              <div className="flex items-center gap-2">
                <FileText className="h-4 w-4 text-muted-foreground" />
                <span className="text-sm font-medium">
                  Changed files ({diffSummary.files.length})
                </span>
              </div>
              <div className="flex items-center gap-3">
                <span className="text-sm text-green-600">
                  +{diffSummary.totalAdditions}
                </span>
                <span className="text-sm text-red-600">
                  -{diffSummary.totalDeletions}
                </span>
                {showFiles ? (
                  <ChevronDown className="h-4 w-4" />
                ) : (
                  <ChevronRight className="h-4 w-4" />
                )}
              </div>
            </button>

            {showFiles && (
              <div className="border-t divide-y">
                {diffSummary.files.map((file, i) => (
                  <div
                    key={i}
                    className="flex items-center justify-between p-2 px-3 text-sm hover:bg-muted/30"
                  >
                    <span className="font-mono text-xs truncate flex-1">
                      {file.path}
                    </span>
                    <div className="flex items-center gap-2 flex-shrink-0 ml-2">
                      <span className="text-green-600 flex items-center gap-0.5">
                        <Plus className="h-3 w-3" />
                        {file.additions}
                      </span>
                      <span className="text-red-600 flex items-center gap-0.5">
                        <Minus className="h-3 w-3" />
                        {file.deletions}
                      </span>
                    </div>
                  </div>
                ))}
              </div>
            )}
          </div>

          {/* Actions */}
          <div className="flex items-center justify-end gap-2 pt-2">
            <Button
              size="sm"
              variant="outline"
              onClick={() => setShowRequestChanges(true)}
            >
              <MessageSquare className="h-4 w-4 mr-1" />
              <span className="hidden sm:inline">Request Changes</span>
            </Button>
            <Button
              size="sm"
              variant="outline"
              className="text-red-600 hover:text-red-700 hover:bg-red-50"
              onClick={() => onReject(approval.id)}
            >
              <X className="h-4 w-4 mr-1" />
              <span className="hidden sm:inline">Decline</span>
            </Button>
            <Button
              size="sm"
              className="bg-green-600 hover:bg-green-700 text-white"
              onClick={() => setShowApproveDialog(true)}
            >
              <Check className="h-4 w-4 mr-1" />
              Accept
            </Button>
          </div>
        </div>
      </Card>

      {/* Approve Dialog */}
      <Dialog open={showApproveDialog} onOpenChange={setShowApproveDialog}>
        <DialogContent className="max-w-lg">
          <DialogHeader>
            <DialogTitle>Accept Changes</DialogTitle>
          </DialogHeader>
          <div className="py-4 space-y-4">
            <p className="text-sm text-muted-foreground">
              This will keep the commits on branch{" "}
              <code className="bg-muted px-1 py-0.5 rounded">
                {approval.branchName}
              </code>
              .
            </p>

            <PluginActionCheckboxes
              type="approval"
              selectedActions={selectedPluginActions}
              onSelectionChange={setSelectedPluginActions}
            />

            <div className="space-y-2">
              <label className="text-sm font-medium">
                Documentation (optional)
              </label>
              <Textarea
                placeholder="e.g. &quot;Document the auth middleware&quot;"
                value={docRequest}
                onChange={(e) => setDocRequest(e.target.value)}
                rows={3}
              />
              <p className="text-xs text-muted-foreground">
                If provided, an agent will create documentation after acceptance.
              </p>
            </div>
          </div>
          <DialogFooter>
            <Button
              variant="outline"
              onClick={() => setShowApproveDialog(false)}
            >
              Cancel
            </Button>
            <Button
              onClick={handleApprove}
              disabled={loading}
              className="bg-green-600 hover:bg-green-700 text-white"
            >
              {loading ? "Accepting..." : "Accept"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* Request Changes Dialog */}
      <Dialog open={showRequestChanges} onOpenChange={setShowRequestChanges}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Request Changes</DialogTitle>
          </DialogHeader>
          <div className="py-4">
            <Textarea
              placeholder="Describe what the agent should fix or change..."
              value={instructions}
              onChange={(e) => setInstructions(e.target.value)}
              rows={4}
            />
          </div>
          <DialogFooter>
            <Button
              variant="outline"
              onClick={() => setShowRequestChanges(false)}
            >
              Cancel
            </Button>
            <Button
              onClick={handleRequestChanges}
              disabled={!instructions.trim() || loading}
            >
              {loading ? "Sending..." : "Send to Agent"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  );
}
