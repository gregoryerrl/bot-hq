"use client";

import { useState, useEffect, useCallback } from "react";
import { useRouter } from "next/navigation";
import { Header } from "@/components/layout/header";
import { ProjectCard } from "@/components/projects/project-card";
import { useCommandContext } from "@/components/command-bar/command-context";
import { Loader2 } from "lucide-react";

interface ProjectData {
  id: number;
  name: string;
  description: string | null;
  status: string;
  taskCounts?: Record<string, number>;
  diagramCount?: number;
  updatedAt: string;
}

export default function DashboardPage() {
  const router = useRouter();
  const { clearContext } = useCommandContext();
  const [projects, setProjects] = useState<ProjectData[]>([]);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    clearContext();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const fetchProjects = useCallback(async () => {
    try {
      const res = await fetch("/api/projects?status=active");
      if (res.ok) {
        const data = await res.json();
        setProjects(data.slice(0, 6));
      }
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    fetchProjects();
  }, [fetchProjects]);

  const handleProjectClick = (id: number) => {
    router.push(`/projects/${id}`);
  };

  return (
    <div className="flex flex-col h-full">
      <Header title="Dashboard" description="Recent projects" />
      <div className="flex-1 p-4 md:p-6 overflow-auto">
        {loading ? (
          <div className="flex items-center justify-center py-12">
            <Loader2 className="h-6 w-6 animate-spin text-muted-foreground" />
          </div>
        ) : projects.length === 0 ? (
          <div className="flex items-center justify-center py-12">
            <p className="text-muted-foreground">
              No projects yet. Go to Projects to create one.
            </p>
          </div>
        ) : (
          <div className="grid sm:grid-cols-2 lg:grid-cols-3 gap-4">
            {projects.map((project) => (
              <ProjectCard
                key={project.id}
                id={project.id}
                name={project.name}
                description={project.description}
                status={project.status}
                taskCounts={project.taskCounts}
                diagramCount={project.diagramCount}
                updatedAt={project.updatedAt}
                onClick={handleProjectClick}
              />
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
