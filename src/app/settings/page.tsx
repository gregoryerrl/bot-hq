"use client";

import { useState } from "react";
import { Header } from "@/components/layout/header";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { WorkspaceList } from "@/components/settings/workspace-list";
import { AddWorkspaceDialog } from "@/components/settings/add-workspace-dialog";
import { DeviceList } from "@/components/settings/device-list";
import { PairingDisplay } from "@/components/settings/pairing-display";

export default function SettingsPage() {
  const [dialogOpen, setDialogOpen] = useState(false);
  const [refreshKey, setRefreshKey] = useState(0);

  return (
    <div className="flex flex-col h-full">
      <Header
        title="Settings"
        description="Configure workspaces and devices"
      />
      <div className="flex-1 p-6">
        <Tabs defaultValue="workspaces" className="space-y-6">
          <TabsList>
            <TabsTrigger value="workspaces">Workspaces</TabsTrigger>
            <TabsTrigger value="devices">Devices</TabsTrigger>
          </TabsList>

          <TabsContent value="workspaces" className="space-y-6">
            <WorkspaceList
              key={refreshKey}
              onAddClick={() => setDialogOpen(true)}
            />
          </TabsContent>

          <TabsContent value="devices" className="space-y-6">
            <PairingDisplay />
            <DeviceList />
          </TabsContent>
        </Tabs>
      </div>

      <AddWorkspaceDialog
        open={dialogOpen}
        onOpenChange={setDialogOpen}
        onSuccess={() => setRefreshKey((k) => k + 1)}
      />
    </div>
  );
}
