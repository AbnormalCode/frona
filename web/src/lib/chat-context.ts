"use client";

import { createContext, useContext, createElement } from "react";

interface ChatContextValue {
  chatId: string;
  agentId: string;
  /** True when this chat was shared with the viewer (read-only) rather than
   *  owned by them - composer and HITL resolution are hidden. The backend
   *  independently rejects mutations from non-owners; this only drives UI. */
  isReadOnly?: boolean;
}

export const ChatContext = createContext<ChatContextValue | null>(null);

export function ChatProvider({
  chatId,
  agentId,
  isReadOnly,
  children,
}: {
  chatId: string;
  agentId: string;
  isReadOnly?: boolean;
  children: React.ReactNode;
}) {
  return createElement(
    ChatContext.Provider,
    { value: { chatId, agentId, isReadOnly } },
    children,
  );
}

export function useChat(): ChatContextValue {
  const ctx = useContext(ChatContext);
  if (!ctx) throw new Error("useChat must be used within ChatProvider");
  return ctx;
}
