import { Badge } from "@/components/ui/badge";
import { Log } from "@/lib/db/schema";

interface LogEntryProps {
  log: Log & { workspaceName?: string; taskTitle?: string };
}

const typeColors: Record<string, string> = {
  agent: "bg-blue-500",
  test: "bg-purple-500",
  sync: "bg-green-500",
  approval: "bg-yellow-500",
  error: "bg-red-500",
  health: "bg-gray-500",
};

export function LogEntry({ log }: LogEntryProps) {
  const time = new Date(log.createdAt).toLocaleTimeString();

  return (
    <div className="flex items-start gap-3 py-2 border-b last:border-0">
      <span className="text-xs text-muted-foreground w-20 flex-shrink-0">
        {time}
      </span>
      <Badge
        variant="secondary"
        className={`${typeColors[log.type]} text-white text-xs`}
      >
        {log.type}
      </Badge>
      <span className="flex-1 text-sm">{log.message}</span>
    </div>
  );
}
