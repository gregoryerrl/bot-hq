import { Header } from "@/components/layout/header";

export default function LogsPage() {
  return (
    <div className="flex flex-col h-full">
      <Header
        title="Logs"
        description="Real-time activity stream"
      />
      <div className="flex-1 p-6">
        <div className="rounded-lg border border-dashed p-8 text-center text-muted-foreground">
          No logs yet
        </div>
      </div>
    </div>
  );
}
