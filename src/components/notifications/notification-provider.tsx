"use client";

import { createContext, useContext, useEffect, useRef, ReactNode } from "react";
import { useNotifications } from "@/hooks/use-notifications";

type NotificationContextType = ReturnType<typeof useNotifications>;

const NotificationContext = createContext<NotificationContextType | null>(null);

export function useNotificationContext() {
  const context = useContext(NotificationContext);
  if (!context) {
    throw new Error(
      "useNotificationContext must be used within NotificationProvider"
    );
  }
  return context;
}

interface NotificationProviderProps {
  children: ReactNode;
}

export function NotificationProvider({ children }: NotificationProviderProps) {
  const notifications = useNotifications();
  const addNotificationRef = useRef(notifications.addNotification);

  // Keep ref updated
  useEffect(() => {
    addNotificationRef.current = notifications.addNotification;
  }, [notifications.addNotification]);

  // Listen for log stream events that warrant notifications
  useEffect(() => {
    const eventSource = new EventSource("/api/logs/stream");

    eventSource.onmessage = (event) => {
      try {
        const data = JSON.parse(event.data);
        if (data.type === "connected") return;

        // Notify on important events
        if (data.type === "approval") {
          addNotificationRef.current(
            "Approval Required",
            data.message,
            "warning"
          );
        } else if (data.type === "error") {
          addNotificationRef.current("Error", data.message, "error");
        }
      } catch {
        // Ignore parse errors
      }
    };

    return () => eventSource.close();
  }, []); // Empty deps - EventSource only created once

  return (
    <NotificationContext.Provider value={notifications}>
      {children}
    </NotificationContext.Provider>
  );
}
