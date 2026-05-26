import { cn } from "../lib/cn";

const colorByAuthor: Record<string, string> = {
  brian: "text-author-brian",
  rain: "text-author-rain",
  emma: "text-author-emma",
  user: "text-author-user",
  system: "text-neutral-500",
};

export function authorColorClass(author: string) {
  return colorByAuthor[author] ?? "text-neutral-300";
}

export function AuthorBadge({ author }: { author: string }) {
  return (
    <span
      className={cn(
        "inline-flex items-center gap-1 text-xs font-semibold uppercase tracking-wide",
        authorColorClass(author),
      )}
    >
      <span className="size-1.5 rounded-full bg-current" />
      {author}
    </span>
  );
}
