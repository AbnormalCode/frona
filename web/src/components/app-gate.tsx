"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import { useAuth } from "@/lib/auth";
import { BootLoadError, useNavigationRaw } from "@/lib/navigation-context";
import { ApiError } from "@/lib/api-client";
import { BootError, BootSpinner, RequireAuth } from "@/components/require-auth";

/// When this renders children, the user is authed, connection is online,
/// and all nav boot fields (`agents`, `spaces`, `tasks`, `contacts`) are
/// defined.
export function AppGate({ children }: { children: React.ReactNode }) {
  return (
    <RequireAuth>
      <NavigationGate>{children}</NavigationGate>
    </RequireAuth>
  );
}

function NavigationGate({ children }: { children: React.ReactNode }) {
  const { user } = useAuth();
  const { agents, spaces, tasks, contacts, refresh } = useNavigationRaw();

  const refreshRef = useRef(refresh);
  refreshRef.current = refresh;
  const [failure, setFailure] = useState<BootLoadError | null>(null);

  const load = useCallback(() => {
    setFailure(null);
    refreshRef.current().catch((error: unknown) => {
      // Network failures flip connectionState to "offline" via auth.ts;
      // RequireAuth handles the retry. A server that answered with an error
      // will answer the same way again, so say so instead of spinning forever.
      const cause = error instanceof BootLoadError ? error.cause : error;
      if (cause instanceof ApiError && cause.kind === "unavailable") return;
      setFailure(
        error instanceof BootLoadError
          ? error
          : new BootLoadError("workspace", error),
      );
    });
  }, []);

  useEffect(() => {
    if (!user) return;
    load();
  }, [user, load]);

  const dataReady =
    agents !== undefined &&
    spaces !== undefined &&
    tasks !== undefined &&
    contacts !== undefined;

  if (dataReady) {
    return <>{children}</>;
  }

  if (failure) {
    return <BootError what={failure.what} message={failure.message} onRetry={load} />;
  }

  const pending: string[] = [];
  if (agents === undefined) pending.push("agents");
  if (spaces === undefined) pending.push("chats");
  if (tasks === undefined) pending.push("tasks");
  if (contacts === undefined) pending.push("contacts");
  const subtitle =
    pending.length === 0
      ? "Loading your workspace…"
      : `Loading ${pending.join(", ")}…`;

  return <BootSpinner subtitle={subtitle} />;
}
