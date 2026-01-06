import { Header } from "@/components/layout/header";

export default function TaskboardPage() {
  return (
    <div className="flex flex-col h-full">
      <Header
        title="Taskboard"
        description="Manage issues across all repositories"
      />
      <div className="flex-1 p-6">
        <div className="rounded-lg border border-dashed p-8 text-center text-muted-foreground">
          No workspaces configured. Add a workspace in Settings.
        </div>
      </div>
    </div>
  );
}
