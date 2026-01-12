"use client";

import { useState, useEffect } from "react";
import { DraftPRCard } from "./draft-pr-card";
import { Approval } from "@/lib/db/schema";

export function ApprovalList() {
  const [approvals, setApprovals] = useState<
    (Approval & {
      taskTitle?: string;
      taskId?: number;
      workspaceName?: string;
    })[]
  >([]);
  const [loading, setLoading] = useState(true);

  async function fetchApprovals() {
    try {
      const res = await fetch("/api/approvals?status=pending");
      const data = await res.json();
      setApprovals(data);
    } catch (error) {
      console.error("Failed to fetch approvals:", error);
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    fetchApprovals();
    const interval = setInterval(fetchApprovals, 3000);
    return () => clearInterval(interval);
  }, []);

  async function handleApprove(id: number, docRequest?: string, pluginActions?: string[]) {
    try {
      await fetch(`/api/approvals/${id}`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ action: "approve", docRequest, pluginActions }),
      });
      fetchApprovals();
    } catch (error) {
      console.error("Failed to approve:", error);
    }
  }

  async function handleReject(id: number) {
    try {
      await fetch(`/api/approvals/${id}`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ action: "reject" }),
      });
      fetchApprovals();
    } catch (error) {
      console.error("Failed to reject:", error);
    }
  }

  async function handleRequestChanges(id: number, instructions: string) {
    try {
      await fetch(`/api/approvals/${id}`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ action: "request_changes", instructions }),
      });
      fetchApprovals();
    } catch (error) {
      console.error("Failed to request changes:", error);
    }
  }

  if (loading) {
    return <div className="text-muted-foreground">Loading approvals...</div>;
  }

  if (approvals.length === 0) {
    return (
      <div className="rounded-lg border border-dashed p-8 text-center text-muted-foreground">
        No pending approvals. Agent work will appear here when complete.
      </div>
    );
  }

  return (
    <div className="space-y-4">
      {approvals.map((approval) => (
        <DraftPRCard
          key={approval.id}
          approval={approval}
          onApprove={handleApprove}
          onReject={handleReject}
          onRequestChanges={handleRequestChanges}
        />
      ))}
    </div>
  );
}
