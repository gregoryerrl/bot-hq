import { Header } from "@/components/layout/header";

export default function PendingPage() {
  return (
    <div className="flex flex-col h-full">
      <Header
        title="Pending Approvals"
        description="Actions waiting for your approval"
      />
      <div className="flex-1 p-6">
        <div className="rounded-lg border border-dashed p-8 text-center text-muted-foreground">
          No pending approvals
        </div>
      </div>
    </div>
  );
}
