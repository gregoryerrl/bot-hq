import { Header } from "@/components/layout/header";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { GitBranch } from "lucide-react";

export default function PendingPage() {
  return (
    <div className="flex flex-col h-full">
      <Header
        title="Review"
        description="Review completed work from tasks"
      />
      <div className="flex-1 p-4 md:p-6">
        <Card>
          <CardHeader>
            <CardTitle className="flex items-center gap-2">
              <GitBranch className="h-5 w-5" />
              Git-Native Review
            </CardTitle>
          </CardHeader>
          <CardContent>
            <p className="text-muted-foreground">
              The review system is being migrated to a git-native workflow.
              Tasks that complete work will create branches that can be reviewed
              directly through the task board.
            </p>
          </CardContent>
        </Card>
      </div>
    </div>
  );
}
