import { Header } from "@/components/layout/header";

export default function SettingsPage() {
  return (
    <div className="flex flex-col h-full">
      <Header
        title="Settings"
        description="Configure workspaces and devices"
      />
      <div className="flex-1 p-6">
        <div className="rounded-lg border border-dashed p-8 text-center text-muted-foreground">
          Settings coming soon
        </div>
      </div>
    </div>
  );
}
