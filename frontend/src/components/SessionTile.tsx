import { Link } from "react-router-dom";
import { cn } from "../lib/cn";
import type { SessionInfo } from "../lib/bindings";

interface SessionTileProps {
  session: SessionInfo;
  needsInput?: boolean;
  lastActivity?: string;
}

export function SessionTile({
  session,
  needsInput = false,
  lastActivity,
}: SessionTileProps) {
  return (
    <Link
      to={`/sessions/${session.id}`}
      className={cn(
        "group block rounded-lg border bg-surface p-4 transition-all duration-150",
        "hover:bg-elevated hover:shadow-lg",
        needsInput
          ? "border-red-500/60 hover:border-red-400/80"
          : "border-default hover:border-neutral-700",
      )}
    >
      <div className="flex items-start justify-between gap-2">
        <h3 className="text-sm font-semibold text-neutral-100 line-clamp-2 group-hover:text-white">
          {session.title}
        </h3>
        {needsInput && (
          <span className="inline-flex shrink-0 items-center gap-1 rounded-full bg-red-500/15 px-2 py-0.5 text-[0.65rem] font-semibold uppercase tracking-wide text-red-300">
            <span className="size-1.5 animate-pulse rounded-full bg-red-400" />
            Needs Input
          </span>
        )}
      </div>
      <div className="mt-3 flex items-center justify-between text-[0.7rem] text-neutral-500">
        <code className="font-mono text-[0.65rem] text-neutral-600">
          {session.id.slice(0, 8)}
        </code>
        <span>{lastActivity ?? formatRelative(session.created_at)}</span>
      </div>
    </Link>
  );
}

function formatRelative(iso: string): string {
  if (!iso) return "";
  const then = new Date(iso).getTime();
  if (!Number.isFinite(then)) return iso;
  const now = Date.now();
  const sec = Math.max(0, Math.floor((now - then) / 1000));
  if (sec < 60) return `${sec}s ago`;
  const min = Math.floor(sec / 60);
  if (min < 60) return `${min}m ago`;
  const hr = Math.floor(min / 60);
  if (hr < 24) return `${hr}h ago`;
  const day = Math.floor(hr / 24);
  return `${day}d ago`;
}
