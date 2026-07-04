import { createBrowserRouter, RouterProvider } from "react-router-dom";
import { Shell } from "./app/Shell";
import { Dashboard } from "./app/Dashboard";
import { SessionView } from "./app/SessionView";
import { Settings } from "./app/Settings";
import { ContextLibrary } from "./app/ContextLibrary";
import { PluginManager } from "./app/PluginManager";
import { PluginPanel } from "./app/PluginPanel";

const router = createBrowserRouter([
  {
    path: "/",
    element: <Shell />,
    children: [
      { index: true, element: <Dashboard /> },
      { path: "sessions/:sessionId", element: <SessionView /> },
      { path: "settings", element: <Settings /> },
      { path: "cl", element: <ContextLibrary /> },
      { path: "plugins", element: <PluginManager /> },
      { path: "plugins/view/:pluginId", element: <PluginPanel /> },
    ],
  },
]);

export function Router() {
  return <RouterProvider router={router} />;
}
