"use client";

import { useState, useEffect } from "react";

interface PluginTab {
  pluginName: string;
  id: string;
  label: string;
  icon: string;
  component: string;
}

interface PluginUIState {
  tabs: PluginTab[];
  workspaceSettings: { pluginName: string; component: string }[];
  taskBadges: { pluginName: string; component: string }[];
  taskActions: { pluginName: string; component: string }[];
  loading: boolean;
}

export function usePluginUI() {
  const [state, setState] = useState<PluginUIState>({
    tabs: [],
    workspaceSettings: [],
    taskBadges: [],
    taskActions: [],
    loading: true,
  });

  useEffect(() => {
    async function fetchContributions() {
      try {
        const res = await fetch("/api/plugins/ui");
        if (!res.ok) throw new Error("Failed to fetch");

        const contributions = await res.json();

        const tabs: PluginTab[] = [];
        const workspaceSettings: { pluginName: string; component: string }[] = [];
        const taskBadges: { pluginName: string; component: string }[] = [];
        const taskActions: { pluginName: string; component: string }[] = [];

        for (const contrib of contributions) {
          if (contrib.tabs) {
            for (const tab of contrib.tabs) {
              tabs.push({ ...tab, pluginName: contrib.pluginName });
            }
          }
          if (contrib.workspaceSettings) {
            workspaceSettings.push({
              pluginName: contrib.pluginName,
              component: contrib.workspaceSettings,
            });
          }
          if (contrib.taskBadge) {
            taskBadges.push({
              pluginName: contrib.pluginName,
              component: contrib.taskBadge,
            });
          }
          if (contrib.taskActions) {
            taskActions.push({
              pluginName: contrib.pluginName,
              component: contrib.taskActions,
            });
          }
        }

        setState({
          tabs,
          workspaceSettings,
          taskBadges,
          taskActions,
          loading: false,
        });
      } catch (error) {
        console.error("Failed to load plugin UI:", error);
        setState(prev => ({ ...prev, loading: false }));
      }
    }

    fetchContributions();
  }, []);

  return state;
}
