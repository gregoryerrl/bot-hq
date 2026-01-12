"use client";

import { useState, useEffect, useCallback } from "react";
import { Card } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Input } from "@/components/ui/input";
import { Check, X, Clock, Smartphone, RefreshCw } from "lucide-react";

interface PendingDevice {
  id: number;
  pairingCode: string;
  deviceId: string;
  userAgent: string;
  ip: string;
  requestedAt: string;
  expiresAt: Date;
}

export function PairingDisplay() {
  const [pending, setPending] = useState<PendingDevice[]>([]);
  const [loading, setLoading] = useState(true);
  const [deviceNames, setDeviceNames] = useState<Record<string, string>>({});

  const fetchPending = useCallback(async () => {
    try {
      const res = await fetch("/api/auth/pending");
      if (res.ok) {
        const data = await res.json();
        if (Array.isArray(data)) {
          setPending(data);
        } else {
          console.error("API returned non-array for pending devices:", data);
          setPending([]);
        }
      }
    } catch (error) {
      console.error("Failed to fetch pending:", error);
      setPending([]);
    } finally {
      setLoading(false);
    }
  }, []);

  async function handleApprove(pairingCode: string) {
    const deviceName = deviceNames[pairingCode] || "";
    try {
      const res = await fetch("/api/auth/approve", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ pairingCode, deviceName }),
      });
      if (res.ok) {
        fetchPending();
      }
    } catch (error) {
      console.error("Failed to approve:", error);
    }
  }

  async function handleReject(pairingCode: string) {
    try {
      const res = await fetch("/api/auth/reject", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ pairingCode }),
      });
      if (res.ok) {
        fetchPending();
      }
    } catch (error) {
      console.error("Failed to reject:", error);
    }
  }

  function parseUserAgent(ua: string): string {
    if (ua.includes("iPhone")) return "iPhone";
    if (ua.includes("iPad")) return "iPad";
    if (ua.includes("Android")) return "Android";
    if (ua.includes("Mac")) return "Mac";
    if (ua.includes("Windows")) return "Windows";
    return "Unknown Device";
  }

  useEffect(() => {
    fetchPending();
    // Poll every 3 seconds for new devices
    const interval = setInterval(fetchPending, 3000);
    return () => clearInterval(interval);
  }, [fetchPending]);

  if (loading) {
    return (
      <div className="flex items-center gap-2 text-muted-foreground">
        <RefreshCw className="h-4 w-4 animate-spin" />
        Loading...
      </div>
    );
  }

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <h3 className="text-lg font-semibold">Pending Device Requests</h3>
        <Button variant="ghost" size="sm" onClick={fetchPending}>
          <RefreshCw className="h-4 w-4" />
        </Button>
      </div>

      {pending.length === 0 ? (
        <Card className="p-6 text-center text-muted-foreground">
          <Clock className="h-8 w-8 mx-auto mb-2 opacity-50" />
          <p>No pending device requests</p>
          <p className="text-xs mt-1">
            Devices trying to access Bot-HQ will appear here
          </p>
        </Card>
      ) : (
        <div className="space-y-3">
          {pending.map((device) => (
            <Card key={device.id} className="p-4">
              <div className="space-y-3">
                <div className="flex items-start justify-between">
                  <div className="flex items-center gap-3">
                    <div className="flex h-10 w-10 items-center justify-center rounded-full bg-muted">
                      <Smartphone className="h-5 w-5 text-muted-foreground" />
                    </div>
                    <div>
                      <div className="font-medium">
                        {parseUserAgent(device.userAgent)}
                      </div>
                      <div className="text-sm text-muted-foreground">
                        IP: {device.ip}
                      </div>
                    </div>
                  </div>
                  <Badge variant="outline" className="font-mono text-lg">
                    {device.pairingCode}
                  </Badge>
                </div>

                <div className="flex items-center gap-2">
                  <Input
                    placeholder="Device name (optional)"
                    className="flex-1"
                    value={deviceNames[device.pairingCode] || ""}
                    onChange={(e) =>
                      setDeviceNames({
                        ...deviceNames,
                        [device.pairingCode]: e.target.value,
                      })
                    }
                  />
                  <Button
                    size="sm"
                    variant="outline"
                    className="text-green-600 border-green-600 hover:bg-green-50"
                    onClick={() => handleApprove(device.pairingCode)}
                  >
                    <Check className="h-4 w-4 mr-1" />
                    Approve
                  </Button>
                  <Button
                    size="sm"
                    variant="outline"
                    className="text-red-600 border-red-600 hover:bg-red-50"
                    onClick={() => handleReject(device.pairingCode)}
                  >
                    <X className="h-4 w-4 mr-1" />
                    Reject
                  </Button>
                </div>
              </div>
            </Card>
          ))}
        </div>
      )}
    </div>
  );
}
