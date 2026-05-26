import { Link } from "react-router-dom";
import { cn } from "../lib/cn";
import type { SessionInfo } from "../lib/bindings";

interface SessionTileProps {
  session: SessionInfo;
  needsInput?: boolean;
  lastActivity?: string;
}

export function SessionTile({ session, needsInput = false, lastActivity }: SessionTileProps) {
  return (
    <Link
      to={`/sessions/${session.id}`}
      className={cn(
        "block rounded-lg border bg-neutral-900/50 p-4 transition-colors",
        "hover:bg-neutral-900/80 hover:border-neutral-700",
        needsInput ? "border-red-500/70" : "border-neutral-800",
      )}
    >
      <div className="flex items-start justify-between gap-2">
        <h3 className="text-sm font-semibold text-neutral-100 line-clamp-2">
          {session.title}
        </h3>
        {needsInput && (
          <span className="inline-flex shrink-0 rounded bg-red-500/15 px-1.5 py-0.5 text-[0.65rem] font-semibold uppercase text-red-300">
            Needs Input
          </span>
        )}
      </div>
      <p className="mt-2 text-xs text-neutral-500">
        {lastActivity ?? session.created_at}
      </p>
    </Link>
  );
}
