import { Header } from "@/components/layout/header";
import { ApprovalList } from "@/components/pending-board/approval-list";

export default function PendingPage() {
  return (
    <div className="flex flex-col h-full">
      <Header
        title="Pending Approvals"
        description="Actions waiting for your approval"
      />
      <div className="flex-1 p-4 md:p-6">
        <ApprovalList />
      </div>
    </div>
  );
}
