"use client";

import { useState, useEffect, useCallback, use } from "react";
import { Header } from "@/components/layout/header";
import { FlowCard } from "@/components/diagrams/flow-card";

interface DiagramSummary {
  id: number;
  title: string;
  workspaceId: number;
  flowData: string;
  createdAt: string;
  updatedAt: string;
}

interface WorkspaceInfo {
  id: number;
  name: string;
}

export default function DiagramListPage({
  params,
}: {
  params: Promise<{ id: string }>;
}) {
  const { id } = use(params);
  const [diagrams, setDiagrams] = useState<DiagramSummary[]>([]);
  const [workspace, setWorkspace] = useState<WorkspaceInfo | null>(null);
  const [loading, setLoading] = useState(true);

  const fetchData = useCallback(async () => {
    try {
      const [diagramRes, workspaceRes] = await Promise.all([
        fetch(`/api/diagrams?workspaceId=${id}`),
        fetch(`/api/workspaces/${id}`),
      ]);

      if (diagramRes.ok) {
        setDiagrams(await diagramRes.json());
      }
      if (workspaceRes.ok) {
        setWorkspace(await workspaceRes.json());
      }
    } catch (error) {
      console.error("Failed to fetch diagram data:", error);
    } finally {
      setLoading(false);
    }
  }, [id]);

  useEffect(() => {
    fetchData();
    const interval = setInterval(fetchData, 10000);
    return () => clearInterval(interval);
  }, [fetchData]);

  function handleFlowClick(diagramId: number) {
    window.location.assign(`/workspaces/${id}/diagram/${diagramId}`);
  }

  return (
    <div className="flex flex-col h-full">
      <Header
        title={workspace ? `${workspace.name} — Diagrams` : "Diagrams"}
        description="Interactive user flow diagrams for this workspace"
      />
      <div className="flex-1 p-4 md:p-6">
        {loading ? (
          <div className="text-muted-foreground">Loading diagrams...</div>
        ) : !workspace ? (
          <div className="rounded-lg border border-dashed p-8 text-center text-muted-foreground">
            Workspace not found
          </div>
        ) : diagrams.length === 0 ? (
          <div className="rounded-lg border border-dashed p-8 text-center">
            <p className="text-muted-foreground">
              No diagrams yet for this workspace. Run Startup Operations from the Dashboard to generate them.
            </p>
          </div>
        ) : (
          <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
            {diagrams.map((d) => (
              <FlowCard
                key={d.id}
                id={d.id}
                title={d.title}
                flowData={d.flowData}
                updatedAt={d.updatedAt}
                onClick={handleFlowClick}
              />
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
