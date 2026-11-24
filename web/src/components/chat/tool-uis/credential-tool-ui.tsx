"use client";

import { makeAssistantToolUI } from "@assistant-ui/react";
import { ToolStatusLine, toolPendingIcon } from "./tool-status-line";

export const CredentialToolUI = makeAssistantToolUI<{ prompt: string; url: string; status: string; query: string; reason: string }, string>({
  toolName: "Credential",
  render: ({ args, result, toolCallId }) => (
    <ToolStatusLine
      toolCallId={toolCallId}
      pendingIcon={toolPendingIcon("Credential")}
      label={`Credential: ${args.query}`}
      serverStatus={args.status}
      serverAnswer={args.status === "resolved" ? "Granted" : result ?? null}
    />
  ),
});

export const CredentialsToolUI = makeAssistantToolUI<{ prompt: string; url: string; status: string; items?: { query: string; label?: string | null }[]; reason: string }, string>({
  toolName: "Credentials",
  render: ({ args, result, toolCallId }) => (
    <ToolStatusLine
      toolCallId={toolCallId}
      pendingIcon={toolPendingIcon("Credentials")}
      label={`Credentials: ${(args.items ?? []).map((i) => i.label ?? i.query).join(", ")}`}
      serverStatus={args.status}
      serverAnswer={args.status === "resolved" ? "Granted" : result ?? null}
    />
  ),
});
