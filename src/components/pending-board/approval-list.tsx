"use client";

import { useState, useEffect } from "react";
import { ApprovalCard } from "./approval-card";
import { Approval } from "@/lib/db/schema";

export function ApprovalList() {
  const [approvals, setApprovals] = useState<
    (Approval & {
      taskTitle?: string;
      workspaceName?: string;
      githubIssueNumber?: number;
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

  async function handleAction(id: number, action: "approve" | "reject") {
    try {
      await fetch(`/api/approvals/${id}`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ action }),
      });
      fetchApprovals();
    } catch (error) {
      console.error("Failed to process approval:", error);
    }
  }

  if (loading) {
    return <div className="text-muted-foreground">Loading approvals...</div>;
  }

  if (approvals.length === 0) {
    return (
      <div className="rounded-lg border border-dashed p-8 text-center text-muted-foreground">
        No pending approvals
      </div>
    );
  }

  return (
    <div className="space-y-3">
      {approvals.map((approval) => (
        <ApprovalCard
          key={approval.id}
          approval={approval}
          onApprove={(id) => handleAction(id, "approve")}
          onReject={(id) => handleAction(id, "reject")}
        />
      ))}
    </div>
  );
}
