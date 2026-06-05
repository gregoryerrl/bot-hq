const colorByAuthor: Record<string, string> = {
  brian: "text-author-brian",
  rain: "text-author-rain",
  user: "text-author-user",
  system: "text-on-surface-variant",
};

export function authorColorClass(author: string) {
  return colorByAuthor[author] ?? "text-on-surface-variant";
}
