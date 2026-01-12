"use client";

import { useState, useEffect } from "react";
import { Card } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Smartphone, Trash2 } from "lucide-react";
import { AuthorizedDevice } from "@/lib/db/schema";

export function DeviceList() {
  const [devices, setDevices] = useState<AuthorizedDevice[]>([]);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    fetchDevices();
  }, []);

  async function fetchDevices() {
    try {
      const res = await fetch("/api/auth/devices");
      if (res.ok) {
        const data = await res.json();
        if (Array.isArray(data)) {
          setDevices(data);
        } else {
          console.error("API returned non-array for devices:", data);
          setDevices([]);
        }
      }
    } catch (error) {
      console.error("Failed to fetch devices:", error);
      setDevices([]);
    } finally {
      setLoading(false);
    }
  }

  async function revokeDevice(id: number) {
    if (!confirm("Revoke this device?")) return;

    try {
      await fetch(`/api/auth/devices/${id}`, { method: "DELETE" });
      setDevices(devices.filter((d) => d.id !== id));
    } catch (error) {
      console.error("Failed to revoke device:", error);
    }
  }

  if (loading) {
    return <div className="text-muted-foreground">Loading devices...</div>;
  }

  return (
    <div className="space-y-4">
      <h3 className="text-lg font-semibold">Authorized Devices</h3>

      {devices.length === 0 ? (
        <Card className="p-6 text-center text-muted-foreground">
          No devices authorized
        </Card>
      ) : (
        <div className="space-y-2">
          {devices.map((device) => (
            <Card key={device.id} className="p-4">
              <div className="flex items-center justify-between">
                <div className="flex items-center gap-3">
                  <Smartphone className="h-5 w-5 text-muted-foreground" />
                  <div>
                    <div className="font-medium">{device.deviceName}</div>
                    <div className="text-sm text-muted-foreground">
                      Last seen:{" "}
                      {device.lastSeenAt
                        ? new Date(device.lastSeenAt).toLocaleDateString()
                        : "Never"}
                    </div>
                  </div>
                </div>
                <div className="flex items-center gap-2">
                  {device.isRevoked ? (
                    <Badge variant="destructive">Revoked</Badge>
                  ) : (
                    <Badge variant="secondary">Active</Badge>
                  )}
                  {!device.isRevoked && (
                    <Button
                      variant="ghost"
                      size="icon"
                      onClick={() => revokeDevice(device.id)}
                    >
                      <Trash2 className="h-4 w-4" />
                    </Button>
                  )}
                </div>
              </div>
            </Card>
          ))}
        </div>
      )}
    </div>
  );
}
