"use client";

import { useState } from "react";
import { Card, CardContent } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Trash2, X } from "lucide-react";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogTrigger,
} from "@/components/ui/alert-dialog";
import { toast } from "sonner";

interface CleanupSuggestionCardProps {
  name: string;
  path: string;
  reason: string;
  onAccepted: () => void;
  onDismissed: () => void;
}

export function CleanupSuggestionCard({
  name,
  path: folderPath,
  reason,
  onAccepted,
  onDismissed,
}: CleanupSuggestionCardProps) {
  const [loading, setLoading] = useState(false);

  async function handleDelete() {
    setLoading(true);
    try {
      const res = await fetch("/api/workspaces/discover", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ action: "delete_folder", path: folderPath }),
      });

      if (!res.ok) {
        const data = await res.json();
        throw new Error(data.error || "Failed to move to Trash");
      }

      toast.success(`"${name}" moved to Trash`);
      onAccepted();
    } catch (error) {
      toast.error(error instanceof Error ? error.message : "Failed to move to Trash");
    } finally {
      setLoading(false);
    }
  }

  return (
    <Card>
      <CardContent className="flex items-center justify-between py-3 px-4">
        <div className="flex items-center gap-3 min-w-0">
          <Trash2 className="h-5 w-5 text-muted-foreground flex-shrink-0" />
          <div className="min-w-0">
            <div className="flex items-center gap-2">
              <p className="font-medium truncate">{name}</p>
              <Badge variant="secondary" className="text-xs flex-shrink-0">
                {reason}
              </Badge>
            </div>
            <p className="text-xs text-muted-foreground truncate">{folderPath}</p>
          </div>
        </div>
        <div className="flex items-center gap-2 flex-shrink-0">
          <AlertDialog>
            <AlertDialogTrigger asChild>
              <Button size="sm" variant="destructive" disabled={loading}>
                Move to Trash
              </Button>
            </AlertDialogTrigger>
            <AlertDialogContent>
              <AlertDialogHeader>
                <AlertDialogTitle>Move to Trash?</AlertDialogTitle>
                <AlertDialogDescription>
                  This will move <strong>{name}</strong> to Trash. You can restore it from Trash if needed.
                </AlertDialogDescription>
              </AlertDialogHeader>
              <AlertDialogFooter>
                <AlertDialogCancel>Cancel</AlertDialogCancel>
                <AlertDialogAction onClick={handleDelete}>
                  Move to Trash
                </AlertDialogAction>
              </AlertDialogFooter>
            </AlertDialogContent>
          </AlertDialog>
          <Button size="sm" variant="ghost" onClick={onDismissed}>
            <X className="h-4 w-4" />
          </Button>
        </div>
      </CardContent>
    </Card>
  );
}
