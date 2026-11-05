import { useEffect, useState } from "react";
import { invoke } from "./host";
import type { RecentConversation } from "./my-work";

export function useRecentConversations(scope: string | null, active: boolean, refresh: number) {
  const [view, setView] = useState<{ scope: string | null; items: RecentConversation[]; error: unknown; loaded: boolean }>(
    () => ({ scope, items: [], error: null, loaded: false }),
  );
  const matches = Boolean(scope) && view.scope === scope;
  const error = matches ? view.error : null;
  useEffect(() => {
    if (!scope || !active) return;
    let current = true;
    void invoke<RecentConversation[]>("ai_recent_conversations")
      .then((items) => { if (current) setView({ scope, items, error: null, loaded: true }); })
      .catch((error) => {
        if (current) setView((previous) => ({ scope, items: previous.scope === scope ? previous.items : [], error, loaded: previous.scope === scope && previous.loaded }));
      });
    return () => { current = false; };
  }, [scope, active, refresh]);
  return {
    items: matches ? view.items : [],
    error,
    loading: Boolean(scope) && (!matches || !view.loaded) && error === null,
  };
}
