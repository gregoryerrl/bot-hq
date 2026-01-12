import { Header } from "@/components/layout/header";
import { PluginList } from "@/components/plugins/plugin-list";

export default function PluginsPage() {
  return (
    <div className="flex flex-col h-full">
      <Header
        title="Plugins"
        description="Manage installed plugins and their settings"
      />
      <div className="flex-1 p-4 md:p-6">
        <PluginList />
      </div>
    </div>
  );
}
