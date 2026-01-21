"use client";

import { useState } from "react";
import { Header } from "@/components/layout/header";
import { WorkspaceList } from "@/components/settings/workspace-list";
import { AddWorkspaceDialog } from "@/components/settings/add-workspace-dialog";

export default function WorkspacesPage() {
  const [dialogOpen, setDialogOpen] = useState(false);
  const [refreshKey, setRefreshKey] = useState(0);

  return (
    <div className="flex flex-col h-full">
      <Header
        title="Workspaces"
        description="Manage your project workspaces"
      />
      <div className="flex-1 p-4 md:p-6">
        <WorkspaceList
          key={refreshKey}
          onAddClick={() => setDialogOpen(true)}
        />
      </div>

      <AddWorkspaceDialog
        open={dialogOpen}
        onOpenChange={setDialogOpen}
        onSuccess={() => setRefreshKey((k) => k + 1)}
      />
    </div>
  );
}
