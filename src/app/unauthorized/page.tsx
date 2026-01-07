"use client";

import { useState, useEffect, useCallback } from "react";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Shield, Loader2, CheckCircle } from "lucide-react";

export default function UnauthorizedPage() {
  const [pairingCode, setPairingCode] = useState<string | null>(null);
  const [status, setStatus] = useState<"loading" | "waiting" | "approved" | "error">("loading");
  const [error, setError] = useState<string | null>(null);

  const getDeviceId = useCallback(() => {
    if (typeof window === "undefined") return null;

    let deviceId = localStorage.getItem("bot_hq_device_id");
    if (!deviceId) {
      // Fallback for browsers without crypto.randomUUID
      if (typeof crypto !== "undefined" && crypto.randomUUID) {
        deviceId = crypto.randomUUID();
      } else {
        // Generate a UUID-like string manually
        deviceId = "xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx".replace(/[xy]/g, (c) => {
          const r = (Math.random() * 16) | 0;
          const v = c === "x" ? r : (r & 0x3) | 0x8;
          return v.toString(16);
        });
      }
      localStorage.setItem("bot_hq_device_id", deviceId);
    }
    return deviceId;
  }, []);

  const registerDevice = useCallback(async () => {
    const deviceId = getDeviceId();
    if (!deviceId) return;

    try {
      const response = await fetch("/api/auth/register", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          deviceId,
          userAgent: navigator.userAgent,
        }),
      });

      if (!response.ok) {
        throw new Error("Failed to register device");
      }

      const data = await response.json();
      setPairingCode(data.pairingCode);
      setStatus("waiting");
    } catch (err) {
      setError(err instanceof Error ? err.message : "Unknown error");
      setStatus("error");
    }
  }, [getDeviceId]);

  const pollForApproval = useCallback(async () => {
    const deviceId = getDeviceId();
    if (!deviceId || !pairingCode) return;

    try {
      const response = await fetch(`/api/auth/poll?deviceId=${deviceId}&code=${pairingCode}`);

      if (response.ok) {
        const data = await response.json();

        if (data.status === "approved") {
          setStatus("approved");
          // Cookie is set by the API, redirect after a moment
          setTimeout(() => {
            window.location.href = "/";
          }, 1500);
        }
      }
    } catch {
      // Ignore poll errors, keep polling
    }
  }, [getDeviceId, pairingCode]);

  // Register device on mount
  useEffect(() => {
    registerDevice();
  }, [registerDevice]);

  // Poll for approval every 2 seconds
  useEffect(() => {
    if (status !== "waiting") return;

    const interval = setInterval(pollForApproval, 2000);
    return () => clearInterval(interval);
  }, [status, pollForApproval]);

  return (
    <div className="min-h-screen flex items-center justify-center bg-background p-4">
      <Card className="w-full max-w-md">
        <CardHeader className="text-center">
          <div className="mx-auto mb-4 flex h-16 w-16 items-center justify-center rounded-full bg-muted">
            <Shield className="h-8 w-8 text-muted-foreground" />
          </div>
          <CardTitle className="text-2xl">Device Authorization Required</CardTitle>
          <CardDescription>
            This device needs to be authorized to access Bot-HQ
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-6">
          {status === "loading" && (
            <div className="flex items-center justify-center py-8">
              <Loader2 className="h-8 w-8 animate-spin text-muted-foreground" />
            </div>
          )}

          {status === "waiting" && pairingCode && (
            <>
              <div className="text-center">
                <p className="text-sm text-muted-foreground mb-4">
                  Enter this code on an authorized device:
                </p>
                <div className="font-mono text-4xl font-bold tracking-[0.5em] py-4 px-6 bg-muted rounded-lg">
                  {pairingCode}
                </div>
              </div>
              <div className="flex items-center justify-center gap-2 text-sm text-muted-foreground">
                <Loader2 className="h-4 w-4 animate-spin" />
                Waiting for approval...
              </div>
              <p className="text-xs text-center text-muted-foreground">
                Go to <strong>Settings → Devices</strong> on your Mac to approve this device
              </p>
            </>
          )}

          {status === "approved" && (
            <div className="text-center py-8">
              <CheckCircle className="h-12 w-12 text-green-500 mx-auto mb-4" />
              <p className="text-lg font-medium">Device Authorized!</p>
              <p className="text-sm text-muted-foreground">Redirecting...</p>
            </div>
          )}

          {status === "error" && (
            <div className="text-center py-8">
              <p className="text-destructive">{error}</p>
              <button
                onClick={() => {
                  setStatus("loading");
                  setError(null);
                  registerDevice();
                }}
                className="mt-4 text-sm text-primary underline"
              >
                Try again
              </button>
            </div>
          )}
        </CardContent>
      </Card>
    </div>
  );
}
