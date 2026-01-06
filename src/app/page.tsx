import { Header } from "@/components/layout/header";
import { TaskList } from "@/components/taskboard/task-list";
import { SyncButton } from "@/components/taskboard/sync-button";

export default function TaskboardPage() {
  return (
    <div className="flex flex-col h-full">
      <Header
        title="Taskboard"
        description="Manage issues across all repositories"
      />
      <div className="flex-1 p-6">
        <div className="flex items-center justify-between mb-6">
          <div className="text-sm text-muted-foreground">
            Issues synced from GitHub
          </div>
          <SyncButton />
        </div>
        <TaskList />
      </div>
    </div>
  );
}
