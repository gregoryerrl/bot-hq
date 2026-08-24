import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { errorMessage } from "../hooks/useInvoke";
import { useFocusTrap } from "../hooks/useFocusTrap";
import { useEscapeKey } from "../hooks/useEscapeKey";
import { Markdown } from "./Markdown";
import { ErrorBanner } from "./ErrorBanner";
import { Button } from "./ui/Button";

/** Mirror of the Rust `WorkspaceFile` — raw invoke, so no binding needed. */
interface WorkspaceFile {
  path: string;
  name: string;
  extension: string;
  text: string | null;
  base64: string | null;
  bytes: number;
}

const MIME_BY_EXT: Record<string, string> = {
  png: "image/png",
  jpg: "image/jpeg",
  jpeg: "image/jpeg",
  gif: "image/gif",
  webp: "image/webp",
  svg: "image/svg+xml",
  bmp: "image/bmp",
};

interface FileViewerDialogProps {
  /** Null closes the dialog. */
  target: { sessionId: string; path: string } | null;
  onClose: () => void;
  /** Optional inline text to show instead of reading a file (long commands). */
  inlineTitle?: string;
  inlineText?: string;
  /**
   * Optional line ABOUT the inline text, rendered above it rather than inside
   * it. Kept out of `inlineText` on purpose: the composed system prompt (rc3
   * P1) needs to say "this is bot-hq's appended portion", and a caveat pasted
   * into the prompt body would read as one more instruction the agent was
   * given.
   */
  inlineNote?: string;
}

/**
 * FULL-SCREEN viewer for a file an agent referenced, or for a command too long
 * to read in a tray card.
 *
 * Deliberately full-screen (`inset-0`) rather than the small centred
 * `ConfirmDialog` box: the content this shows — a GitHub issue body, a diff, a
 * screenshot — is the thing the user is being asked to approve, so it gets the
 * whole window. A gate card previously showed only a `--body-file` PATH, which
 * meant approving a body you could not see.
 *
 * Escape and the backdrop close it, matching ConfirmDialog's affordances.
 */
export function FileViewerDialog({
  target,
  onClose,
  inlineTitle,
  inlineText,
  inlineNote,
}: FileViewerDialogProps) {
  const open = target !== null || inlineText !== undefined;
  const trapRef = useFocusTrap<HTMLDivElement>(open);
  useEscapeKey(onClose, open);

  const [file, setFile] = useState<WorkspaceFile | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  // Keyed on the PRIMITIVES, not `target` itself: callers pass an object
  // literal, so a new identity every render would re-fire the read in a loop.
  const targetSession = target?.sessionId;
  const targetPath = target?.path;

  useEffect(() => {
    if (!targetSession || !targetPath) {
      setFile(null);
      setError(null);
      return;
    }
    let cancelled = false;
    setLoading(true);
    setError(null);
    void (async () => {
      try {
        const f = await invoke<WorkspaceFile>("read_workspace_file", {
          sessionId: targetSession,
          path: targetPath,
        });
        if (!cancelled) setFile(f);
      } catch (e) {
        if (!cancelled) setError(errorMessage(e));
      } finally {
        if (!cancelled) setLoading(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [targetSession, targetPath]);

  if (!open) return null;

  const title = inlineTitle ?? file?.name ?? target?.path ?? "File";
  const mime = file ? MIME_BY_EXT[file.extension] : undefined;

  return (
    <>
      <div
        className="fixed inset-0 z-40 bg-black/70"
        onClick={onClose}
        aria-hidden
      />
      <div
        ref={trapRef}
        tabIndex={-1}
        role="dialog"
        aria-modal="true"
        aria-label={title}
        className="fixed inset-0 z-50 flex flex-col bg-surface-container focus:outline-none"
      >
        <header className="flex shrink-0 items-center gap-3 border-b border-outline-variant px-4 py-3">
          <h2 className="min-w-0 flex-1 truncate font-headline-md text-headline-md text-on-surface">
            {title}
          </h2>
          {file && (
            <span className="shrink-0 font-label-caps text-label-caps text-on-surface-variant">
              {file.bytes.toLocaleString()} bytes
            </span>
          )}
          <Button variant="ghost" onClick={onClose} aria-label="Close viewer">
            Close
          </Button>
        </header>

        {/* The pair, not a bare `overflow-auto`: CSS computes an unspecified
            `overflow-x` to `auto` once the other axis is non-visible, so this
            scrolled sideways. House rule is vertical-only, everywhere. */}
        <div className="min-h-0 flex-1 overflow-y-auto overflow-x-hidden px-4 py-3">
          {error && (
            <ErrorBanner
              label="Couldn't open this file:"
              message={error}
              onDismiss={() => setError(null)}
            />
          )}
          {loading && (
            <p className="font-body-md text-body-md text-on-surface-variant">
              Reading…
            </p>
          )}

          {inlineNote && (
            <p className="mb-3 border-b border-outline-variant pb-2 font-code-sm text-code-sm text-on-surface-variant">
              {inlineNote}
            </p>
          )}

          {inlineText !== undefined && (
            <pre className="whitespace-pre-wrap break-words font-mono text-sm leading-relaxed text-on-surface">
              {inlineText}
            </pre>
          )}

          {file?.base64 && mime && (
            <img
              src={`data:${mime};base64,${file.base64}`}
              alt={file.name}
              className="mx-auto max-h-full max-w-full object-contain"
            />
          )}

          {file?.text !== null && file?.text !== undefined && (
            file.extension === "md" || file.extension === "markdown" ? (
              <Markdown>{file.text}</Markdown>
            ) : (
              <pre className="whitespace-pre-wrap break-words font-mono text-sm leading-relaxed text-on-surface">
                {file.text}
              </pre>
            )
          )}

          {file && (
            <p className="mt-4 border-t border-outline-variant pt-2 font-label-caps text-label-caps text-on-surface-variant">
              {file.path}
            </p>
          )}
        </div>
      </div>
    </>
  );
}

/** Strip the shell quoting a command assembles paths with. */
function stripQuotes(token: string): string {
  return token.replace(/^["']|["']$/g, "");
}

/**
 * The bare-argument extension families agents actually gate: the original
 * body/image set plus scripts, source, config and data files. `.php` was the
 * incident (gate `cc7bf76e`, 2026-08-24): `php /tmp/349-duplicate.php` offered
 * no View button, and the user approved a script they could not read.
 */
const BARE_FILE_EXTS =
  "md|markdown|png|jpe?g|gif|webp|svg|txt|json|diff|patch|" +
  "php|sql|sh|bash|zsh|py|rb|pl|js|mjs|ts|tsx|jsx|yml|yaml|toml|csv|" +
  "html|css|xml|log|conf|ini|env|rs|go|java|c|h|cpp";

/**
 * EVERY file path a gated command names, so the card can offer each one.
 *
 * Flag forms (`--body-file X`, `--file X`, `-F X`) come first — they are the
 * deliberate "here is the body" convention — then bare arguments with a known
 * extension, in appearance order. Returning ALL candidates instead of the
 * first is the point: widening the extension set turned a missing button into
 * a possibly WRONG one (`php -d x=y foo.ini run.php` has two file-shaped
 * arguments, and guessing one hides the other — the same failure shape as the
 * incident). Flag-looking tokens (`-…`) and URLs (`…://…`) are skipped;
 * duplicates collapse.
 */
export function fileArgsInCommand(command: string): string[] {
  const out: string[] = [];
  const flagRe = /(?:--body-file|--file|-F)[=\s]+("[^"]+"|'[^']+'|\S+)/g;
  for (const m of command.matchAll(flagRe)) out.push(stripQuotes(m[1]));
  const bareRe = new RegExp(`(\\S+\\.(?:${BARE_FILE_EXTS}))\\b`, "gi");
  for (const m of command.matchAll(bareRe)) {
    const token = stripQuotes(m[1]);
    if (token.startsWith("-")) continue;
    if (token.includes("://")) continue;
    out.push(token);
  }
  return [...new Set(out)];
}

/**
 * The single most likely file a command names — the first candidate. Kept for
 * single-slot surfaces (the chat tool pill); gate and tray cards render every
 * candidate via [`fileArgsInCommand`].
 */
export function fileArgInCommand(command: string): string | null {
  return fileArgsInCommand(command)[0] ?? null;
}
