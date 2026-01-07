import { Header } from "@/components/layout/header";
import { LogDetail } from "@/components/log-viewer/log-detail";

export default function ServerLogsPage() {
  return (
    <div className="flex flex-col h-full">
      <Header title="Server Logs" description="System and infrastructure logs" />
      <div className="flex-1 p-4 md:p-6">
        <LogDetail
          title="Server Logs"
          source="server"
          subtitle="Sync, health, approval, and error logs"
        />
      </div>
    </div>
  );
}
