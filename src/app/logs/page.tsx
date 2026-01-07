import { Header } from "@/components/layout/header";
import { LogSourceList } from "@/components/log-viewer/log-source-list";

export default function LogsPage() {
  return (
    <div className="flex flex-col h-full">
      <Header title="Logs" description="Real-time activity streams" />
      <div className="flex-1 p-4 md:p-6">
        <LogSourceList />
      </div>
    </div>
  );
}
