import { useEffect, useRef, useState } from "react";

/**
 * Mirror a server snapshot into editable local draft state. Re-hydrates the
 * draft whenever the server value changes (initial load + post-save refetch),
 * comparing by JSON so a structurally-equal refetch doesn't clobber in-progress
 * edits. `dirty` is true while the draft diverges from the server.
 *
 * Callers pass the already-defaulted value (e.g. `server ?? {}`).
 */
export function useServerDraft<T>(server: T) {
  const serverJson = JSON.stringify(server);
  const [draft, setDraft] = useState<T>(server);
  const lastServer = useRef(serverJson);
  useEffect(() => {
    if (lastServer.current !== serverJson) {
      lastServer.current = serverJson;
      setDraft(server);
    }
  }, [serverJson, server]);
  const dirty = JSON.stringify(draft) !== serverJson;
  return { draft, setDraft, dirty };
}
