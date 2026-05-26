import { createBrowserRouter, RouterProvider } from "react-router-dom";
import { Shell } from "./app/Shell";
import { Dashboard } from "./app/Dashboard";
import { SessionView } from "./app/SessionView";
import { Settings } from "./app/Settings";
import { ContextLibrary } from "./app/ContextLibrary";
import { PluginManager } from "./app/PluginManager";

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
    ],
  },
]);

export function Router() {
  return <RouterProvider router={router} />;
}
