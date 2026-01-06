import { Header } from "@/components/layout/header";
import { LogList } from "@/components/log-viewer/log-list";

export default function LogsPage() {
  return (
    <div className="flex flex-col h-full">
      <Header title="Logs" description="Real-time activity stream" />
      <div className="flex-1 p-6">
        <LogList />
      </div>
    </div>
  );
}
