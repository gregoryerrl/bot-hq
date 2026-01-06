"use client";

import { useState, useEffect } from "react";
import { Card } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { RefreshCw, Check, X } from "lucide-react";

interface PendingPairing {
  code: string;
  deviceName: string;
  fingerprint: string;
  expiresAt: string;
}

export function PairingDisplay() {
  const [code, setCode] = useState<string | null>(null);
  const [pending, setPending] = useState<PendingPairing[]>([]);
  const [loading, setLoading] = useState(false);

  async function fetchCode() {
    setLoading(true);
    try {
      const res = await fetch("/api/auth/pair");
      const data = await res.json();
      setCode(data.code);
    } catch (error) {
      console.error("Failed to fetch pairing code:", error);
    } finally {
      setLoading(false);
    }
  }

  async function fetchPending() {
    try {
      const res = await fetch("/api/auth/pair?action=pending");
      const data = await res.json();
      setPending(data.pending || []);
    } catch (error) {
      console.error("Failed to fetch pending:", error);
    }
  }

  async function handleApprove(fingerprint: string) {
    try {
      await fetch("/api/auth/pair", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ action: "approve", fingerprint }),
      });
      fetchPending();
    } catch (error) {
      console.error("Failed to approve:", error);
    }
  }

  async function handleReject(fingerprint: string) {
    try {
      await fetch("/api/auth/pair", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ action: "reject", fingerprint }),
      });
      fetchPending();
    } catch (error) {
      console.error("Failed to reject:", error);
    }
  }

  useEffect(() => {
    fetchCode();
    fetchPending();
    const interval = setInterval(fetchPending, 5000);
    return () => clearInterval(interval);
  }, []);

  return (
    <div className="space-y-4">
      <h3 className="text-lg font-semibold">Device Pairing</h3>

      <Card className="p-6">
        <div className="text-center space-y-4">
          <p className="text-sm text-muted-foreground">
            Enter this code on your new device
          </p>
          <div className="text-4xl font-mono font-bold tracking-widest">
            {code || "------"}
          </div>
          <Button
            variant="outline"
            size="sm"
            onClick={fetchCode}
            disabled={loading}
          >
            <RefreshCw className={`h-4 w-4 mr-2 ${loading ? "animate-spin" : ""}`} />
            New Code
          </Button>
        </div>
      </Card>

      {pending.length > 0 && (
        <div className="space-y-2">
          <h4 className="text-sm font-medium">Pending Requests</h4>
          {pending.map((p) => (
            <Card key={p.fingerprint} className="p-4">
              <div className="flex items-center justify-between">
                <div>
                  <div className="font-medium">{p.deviceName}</div>
                  <Badge variant="outline" className="text-xs">
                    Waiting for approval
                  </Badge>
                </div>
                <div className="flex gap-2">
                  <Button
                    size="icon"
                    variant="outline"
                    onClick={() => handleApprove(p.fingerprint)}
                  >
                    <Check className="h-4 w-4 text-green-600" />
                  </Button>
                  <Button
                    size="icon"
                    variant="outline"
                    onClick={() => handleReject(p.fingerprint)}
                  >
                    <X className="h-4 w-4 text-red-600" />
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
