"use client";

import { useState } from "react";
import { Header } from "@/components/layout/header";
import { TaskList } from "@/components/taskboard/task-list";
import { SyncButton } from "@/components/taskboard/sync-button";
import { CreateTaskDialog } from "@/components/taskboard/create-task-dialog";
import { Button } from "@/components/ui/button";
import { Plus } from "lucide-react";

export default function TaskboardPage() {
  const [showCreateDialog, setShowCreateDialog] = useState(false);
  const [refreshKey, setRefreshKey] = useState(0);

  const handleTaskCreated = () => {
    setRefreshKey(k => k + 1);
  };

  return (
    <div className="flex flex-col h-full">
      <Header
        title="Taskboard"
        description="Manage tasks across all workspaces"
      />
      <div className="flex-1 p-4 md:p-6">
        <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-4 mb-6">
          <div className="text-sm text-muted-foreground">
            Tasks from all workspaces
          </div>
          <div className="flex items-center gap-2">
            <Button
              size="sm"
              onClick={() => setShowCreateDialog(true)}
            >
              <Plus className="h-4 w-4 mr-1" />
              Create Task
            </Button>
            <SyncButton />
          </div>
        </div>
        <TaskList key={refreshKey} />
      </div>

      <CreateTaskDialog
        open={showCreateDialog}
        onClose={() => setShowCreateDialog(false)}
        onTaskCreated={handleTaskCreated}
      />
    </div>
  );
}
