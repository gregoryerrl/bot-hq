"use client";

import { toast } from "sonner";
import { AlertCircle, ChevronDown, Copy } from "lucide-react";
import { useState } from "react";
import { Button } from "@/components/ui/button";

interface PluginErrorDetails {
  error: string;
  code?: string;
  details?: string;
  pluginName?: string;
  actionId?: string;
}

function ErrorToastContent({
  error,
  code,
  details,
  pluginName,
  actionId,
}: PluginErrorDetails) {
  const [expanded, setExpanded] = useState(false);
  const hasDetails = details || code;

  const copyToClipboard = () => {
    const text = [
      `Error: ${error}`,
      code ? `Code: ${code}` : "",
      pluginName ? `Plugin: ${pluginName}` : "",
      actionId ? `Action: ${actionId}` : "",
      details ? `Details: ${details}` : "",
    ]
      .filter(Boolean)
      .join("\n");
    navigator.clipboard.writeText(text);
    toast.success("Error details copied to clipboard");
  };

  return (
    <div className="space-y-2">
      <div className="flex items-start gap-2">
        <AlertCircle className="h-4 w-4 text-destructive mt-0.5 shrink-0" />
        <div className="flex-1 min-w-0">
          <p className="text-sm font-medium">{error}</p>
          {pluginName && (
            <p className="text-xs text-muted-foreground">
              Plugin: {pluginName}
              {actionId && ` / ${actionId}`}
            </p>
          )}
        </div>
      </div>

      {hasDetails && (
        <>
          <Button
            variant="ghost"
            size="sm"
            className="h-6 px-2 text-xs w-full justify-between"
            onClick={() => setExpanded(!expanded)}
          >
            <span>{expanded ? "Hide" : "Show"} details</span>
            <ChevronDown
              className={`h-3 w-3 transition-transform ${expanded ? "rotate-180" : ""}`}
            />
          </Button>

          {expanded && (
            <div className="text-xs bg-muted/50 rounded p-2 space-y-1">
              {code && (
                <p>
                  <span className="text-muted-foreground">Code:</span> {code}
                </p>
              )}
              {details && (
                <p className="break-words">
                  <span className="text-muted-foreground">Details:</span>{" "}
                  {details}
                </p>
              )}
              <Button
                variant="ghost"
                size="sm"
                className="h-6 px-2 text-xs mt-1"
                onClick={copyToClipboard}
              >
                <Copy className="h-3 w-3 mr-1" />
                Copy error info
              </Button>
            </div>
          )}
        </>
      )}
    </div>
  );
}

export function showPluginError(errorData: PluginErrorDetails) {
  toast.error(
    <ErrorToastContent {...errorData} />,
    {
      duration: 8000,
      className: "plugin-error-toast",
    }
  );
}

export function showPluginSuccess(message: string, pluginName?: string) {
  toast.success(
    <div className="space-y-1">
      <p className="text-sm font-medium">{message}</p>
      {pluginName && (
        <p className="text-xs text-muted-foreground">Plugin: {pluginName}</p>
      )}
    </div>,
    { duration: 4000 }
  );
}

// Parse API error response into structured format
export function parsePluginError(
  response: Record<string, unknown>,
  fallbackMessage: string = "An error occurred"
): PluginErrorDetails {
  return {
    error: (response.error as string) || fallbackMessage,
    code: response.code as string | undefined,
    details: response.details as string | undefined,
  };
}
