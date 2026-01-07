"use client";

import { useState, useCallback } from "react";
import { toast } from "sonner";
import {
  type Notification as AppNotification,
  requestNotificationPermission,
  sendBrowserNotification,
} from "@/lib/notifications";

function getInitialPermission(): NotificationPermission {
  if (typeof window !== "undefined" && "Notification" in window) {
    return Notification.permission;
  }
  return "default";
}

export function useNotifications() {
  const [notifications, setNotifications] = useState<AppNotification[]>([]);
  const [permission, setPermission] = useState<NotificationPermission>(getInitialPermission);

  const requestPermission = useCallback(async () => {
    const result = await requestNotificationPermission();
    setPermission(result);
    return result;
  }, []);

  const addNotification = useCallback(
    (
      title: string,
      body: string,
      type: AppNotification["type"] = "info"
    ) => {
      const notification: AppNotification = {
        id: Date.now().toString(),
        title,
        body,
        type,
        timestamp: new Date(),
        read: false,
      };

      setNotifications((prev) => [notification, ...prev].slice(0, 50));

      // Show toast
      const toastFn = type === "error" ? toast.error : type === "success" ? toast.success : toast;
      toastFn(title, { description: body });

      // Send browser notification if permitted
      if (permission === "granted" && document.hidden) {
        sendBrowserNotification(title, body);
      }
    },
    [permission]
  );

  const markAsRead = useCallback((id: string) => {
    setNotifications((prev) =>
      prev.map((n) => (n.id === id ? { ...n, read: true } : n))
    );
  }, []);

  const clearAll = useCallback(() => {
    setNotifications([]);
  }, []);

  const unreadCount = notifications.filter((n) => !n.read).length;

  return {
    notifications,
    unreadCount,
    permission,
    requestPermission,
    addNotification,
    markAsRead,
    clearAll,
  };
}
