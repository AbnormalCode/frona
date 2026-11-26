import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { useEffect } from "react";
import { AssistantRuntimeProvider, type AssistantRuntime } from "@assistant-ui/react";
import { beforeEach, expect, it, vi } from "vitest";
import { useChatRuntime } from "@/lib/use-chat-runtime";
import { ChatProvider } from "@/lib/chat-context";
import { FronaComposer } from "../frona-composer";

const mocks = vi.hoisted(() => ({ sendMessage: vi.fn() }));
vi.mock("@/lib/api-client", () => ({
  api: { get: vi.fn(async () => ({ messages: [], has_more: false })) },
  uploadFile: vi.fn(), sendMessage: mocks.sendMessage, cancelGeneration: vi.fn(),
}));
vi.mock("@/lib/format-time", () => ({ computeTimeMarkers: () => new Map(), useTimezone: () => "UTC" }));
vi.mock("next/navigation", () => ({ useRouter: () => ({ push: vi.fn() }) }));
vi.mock("../file-browser-modal", () => ({ FileBrowserModal: () => null }));
vi.mock("@/lib/use-commands", () => ({ useCommands: () => ({ commands: [], skills: [] }) }));

let runtime: AssistantRuntime;
function Harness({ onSend }: { onSend?: () => void }) {
  const chat = useChatRuntime({ chatId: "chat-1", agentId: "agent-1" });
  useEffect(() => { runtime = chat.runtime; }, [chat.runtime]);
  return (
    <AssistantRuntimeProvider runtime={chat.runtime}>
      <ChatProvider chatId="chat-1" agentId="agent-1" interrupt={chat.interrupt}>
        <FronaComposer onSend={onSend} />
      </ChatProvider>
    </AssistantRuntimeProvider>
  );
}

/** Start a turn that never finishes, so the thread stays "running". */
async function startRunningTurn() {
  mocks.sendMessage.mockReturnValueOnce(new Promise(() => {}));
  act(() => runtime.thread.composer.setText("first request"));
  fireEvent.submit(screen.getByRole("textbox").closest("form")!);
  await screen.findByTitle("Stop");
  mocks.sendMessage.mockClear();
  mocks.sendMessage.mockResolvedValue(undefined);
}

beforeEach(() => {
  vi.clearAllMocks();
  HTMLElement.prototype.scrollIntoView = vi.fn();
});

it("sends guidance to a running agent with the send button and clears the draft", async () => {
  render(<Harness />);
  await startRunningTurn();
  expect(screen.getByText("Send guidance to steer the agent…")).toBeInTheDocument();

  act(() => runtime.thread.composer.setText("  actually use the other file  "));
  fireEvent.click(screen.getByRole("button", { name: "Send guidance" }));

  expect(mocks.sendMessage).toHaveBeenCalledExactlyOnceWith("chat-1", { content: "actually use the other file" });
  expect(runtime.thread.composer.getState().text).toBe("");
  expect(screen.getByTitle("Stop")).toBeInTheDocument();
});

it("routes Enter to the interrupt while the agent runs, but not Shift+Enter", async () => {
  render(<Harness />);
  await startRunningTurn();
  act(() => runtime.thread.composer.setText("change course"));
  const editor = screen.getByRole("textbox");

  fireEvent.keyDown(editor, { key: "Enter", shiftKey: true });
  expect(mocks.sendMessage).not.toHaveBeenCalled();

  fireEvent.keyDown(editor, { key: "Enter" });
  await waitFor(() => expect(mocks.sendMessage).toHaveBeenCalledExactlyOnceWith("chat-1", { content: "change course" }));
});

it("sends nothing for an empty draft", async () => {
  render(<Harness />);
  await startRunningTurn();
  fireEvent.click(screen.getByRole("button", { name: "Send guidance" }));
  fireEvent.keyDown(screen.getByRole("textbox"), { key: "Enter" });
  expect(mocks.sendMessage).not.toHaveBeenCalled();
});

it("offers no guidance button on a standalone composer", async () => {
  const onSend = vi.fn();
  render(<Harness onSend={onSend} />);
  act(() => runtime.thread.composer.setText("hello"));
  expect(screen.queryByRole("button", { name: "Send guidance" })).not.toBeInTheDocument();
});
