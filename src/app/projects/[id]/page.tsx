"use client";

import { use, useEffect, useState, useCallback } from "react";
import { useRouter } from "next/navigation";
import { Header } from "@/components/layout/header";
import { Tabs, TabsList, TabsTrigger, TabsContent } from "@/components/ui/tabs";
import { Card } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Plus } from "lucide-react";
import { toast } from "sonner";
import { TaskList } from "@/components/projects/task-list";
import { FlowCard } from "@/components/diagrams/flow-card";
import { useCommandContext } from "@/components/command-bar/command-context";

interface ProjectData {
  id: number;
  name: string;
  description: string | null;
  repoPath: string | null;
  status: string;
  notes: string | null;
  createdAt: string | Date;
  updatedAt: string | Date;
  taskCounts: Record<string, number>;
  diagramCount: number;
}

interface DiagramData {
  id: number;
  title: string;
  template: string | null;
  createdAt: string | Date;
  updatedAt: string | Date;
}

export default function ProjectDetailPage({
  params,
}: {
  params: Promise<{ id: string }>;
}) {
  const { id } = use(params);
  const projectId = Number(id);
  const router = useRouter();
  const { setContext, clearContext } = useCommandContext();

  const [project, setProject] = useState<ProjectData | null>(null);
  const [diagrams, setDiagrams] = useState<DiagramData[]>([]);
  const [loading, setLoading] = useState(true);
  const [newDiagramTitle, setNewDiagramTitle] = useState("");
  const [creatingDiagram, setCreatingDiagram] = useState(false);
  const [showNewDiagramInput, setShowNewDiagramInput] = useState(false);

  useEffect(() => {
    if (project) {
      setContext({ projectId: project.id, label: project.name });
    }
    return () => clearContext();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [project?.id]);

  const fetchProject = useCallback(async () => {
    try {
      const res = await fetch(`/api/projects/${projectId}`);
      if (res.ok) {
        const data = await res.json();
        setProject(data);
      }
    } finally {
      setLoading(false);
    }
  }, [projectId]);

  const fetchDiagrams = useCallback(async () => {
    const res = await fetch(`/api/projects/${projectId}/diagrams`);
    if (res.ok) {
      const data = await res.json();
      setDiagrams(data);
    }
  }, [projectId]);

  useEffect(() => {
    fetchProject();
    fetchDiagrams();
  }, [fetchProject, fetchDiagrams]);

  const handleCreateDiagram = async () => {
    if (!newDiagramTitle.trim()) return;

    setCreatingDiagram(true);
    try {
      const res = await fetch(`/api/projects/${projectId}/diagrams`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ title: newDiagramTitle.trim() }),
      });

      if (!res.ok) {
        const data = await res.json();
        throw new Error(data.error || "Failed to create diagram");
      }

      setNewDiagramTitle("");
      setShowNewDiagramInput(false);
      await fetchDiagrams();
    } catch (err) {
      toast.error(
        err instanceof Error ? err.message : "Failed to create diagram"
      );
    } finally {
      setCreatingDiagram(false);
    }
  };

  const handleDiagramClick = (diagramId: number) => {
    router.push(`/projects/${projectId}/visualizer/${diagramId}`);
  };

  if (loading) {
    return (
      <div className="flex flex-col h-full">
        <Header title="Loading..." />
        <div className="flex-1 flex items-center justify-center">
          <p className="text-muted-foreground">Loading project...</p>
        </div>
      </div>
    );
  }

  if (!project) {
    return (
      <div className="flex flex-col h-full">
        <Header title="Not Found" />
        <div className="flex-1 flex items-center justify-center">
          <p className="text-muted-foreground">Project not found.</p>
        </div>
      </div>
    );
  }

  const totalTasks = Object.values(project.taskCounts).reduce(
    (a, b) => a + b,
    0
  );

  return (
    <div className="flex flex-col h-full">
      <Header title={project.name} description={project.description ?? undefined} />
      <div className="flex-1 p-4 md:p-6 overflow-auto">
        <Tabs defaultValue="tasks">
          <TabsList>
            <TabsTrigger value="tasks">Tasks</TabsTrigger>
            <TabsTrigger value="visualizers">Visualizers</TabsTrigger>
            <TabsTrigger value="overview">Overview</TabsTrigger>
          </TabsList>

          <TabsContent value="tasks" className="mt-4">
            <TaskList projectId={projectId} />
          </TabsContent>

          <TabsContent value="visualizers" className="mt-4">
            <div className="flex items-center justify-end mb-4">
              {showNewDiagramInput ? (
                <div className="flex items-center gap-2">
                  <Input
                    value={newDiagramTitle}
                    onChange={(e) => setNewDiagramTitle(e.target.value)}
                    placeholder="Diagram title"
                    className="w-[200px]"
                    onKeyDown={(e) => {
                      if (e.key === "Enter") handleCreateDiagram();
                      if (e.key === "Escape") {
                        setShowNewDiagramInput(false);
                        setNewDiagramTitle("");
                      }
                    }}
                    autoFocus
                  />
                  <Button
                    size="sm"
                    onClick={handleCreateDiagram}
                    disabled={creatingDiagram || !newDiagramTitle.trim()}
                  >
                    {creatingDiagram ? "Creating..." : "Create"}
                  </Button>
                  <Button
                    size="sm"
                    variant="outline"
                    onClick={() => {
                      setShowNewDiagramInput(false);
                      setNewDiagramTitle("");
                    }}
                  >
                    Cancel
                  </Button>
                </div>
              ) : (
                <Button
                  size="sm"
                  onClick={() => setShowNewDiagramInput(true)}
                >
                  <Plus className="h-4 w-4" />
                  New Visualizer
                </Button>
              )}
            </div>

            {diagrams.length === 0 ? (
              <div className="flex items-center justify-center py-12">
                <p className="text-muted-foreground">
                  No visualizers yet. Create one to get started.
                </p>
              </div>
            ) : (
              <div className="grid sm:grid-cols-2 lg:grid-cols-3 gap-4">
                {diagrams.map((diagram) => (
                  <FlowCard
                    key={diagram.id}
                    id={diagram.id}
                    title={diagram.title}
                    nodeCount={0}
                    edgeCount={0}
                    groupCount={0}
                    updatedAt={diagram.updatedAt}
                    onClick={handleDiagramClick}
                  />
                ))}
              </div>
            )}
          </TabsContent>

          <TabsContent value="overview" className="mt-4">
            <div className="space-y-6 max-w-2xl">
              {project.description && (
                <Card className="p-4">
                  <h3 className="text-sm font-medium mb-2">Description</h3>
                  <p className="text-sm text-muted-foreground">
                    {project.description}
                  </p>
                </Card>
              )}

              {project.repoPath && (
                <Card className="p-4">
                  <h3 className="text-sm font-medium mb-2">Repository Path</h3>
                  <p className="text-sm text-muted-foreground font-mono">
                    {project.repoPath}
                  </p>
                </Card>
              )}

              {project.notes && (
                <Card className="p-4">
                  <h3 className="text-sm font-medium mb-2">Notes</h3>
                  <p className="text-sm text-muted-foreground whitespace-pre-wrap">
                    {project.notes}
                  </p>
                </Card>
              )}

              <Card className="p-4">
                <h3 className="text-sm font-medium mb-3">Stats</h3>
                <div className="flex flex-wrap gap-3">
                  <Badge variant="outline">
                    {totalTasks} total task{totalTasks !== 1 ? "s" : ""}
                  </Badge>
                  {(project.taskCounts.todo ?? 0) > 0 && (
                    <Badge variant="secondary">
                      {project.taskCounts.todo} todo
                    </Badge>
                  )}
                  {(project.taskCounts.in_progress ?? 0) > 0 && (
                    <Badge variant="secondary">
                      {project.taskCounts.in_progress} in progress
                    </Badge>
                  )}
                  {(project.taskCounts.done ?? 0) > 0 && (
                    <Badge variant="secondary">
                      {project.taskCounts.done} done
                    </Badge>
                  )}
                  {(project.taskCounts.blocked ?? 0) > 0 && (
                    <Badge variant="destructive">
                      {project.taskCounts.blocked} blocked
                    </Badge>
                  )}
                  <Badge variant="outline">
                    {project.diagramCount} diagram
                    {project.diagramCount !== 1 ? "s" : ""}
                  </Badge>
                </div>
              </Card>

              <Card className="p-4">
                <h3 className="text-sm font-medium mb-2">Status</h3>
                <Badge
                  variant={
                    project.status === "active" ? "default" : "secondary"
                  }
                >
                  {project.status}
                </Badge>
              </Card>
            </div>
          </TabsContent>
        </Tabs>
      </div>
    </div>
  );
}
