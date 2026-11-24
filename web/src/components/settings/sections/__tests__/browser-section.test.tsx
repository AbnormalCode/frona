import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { api } from "@/lib/api-client";
import { BrowserSection } from "../browser-section";

vi.mock("@/lib/api-client", () => ({ api: { post: vi.fn() } }));

const browser = {
  ws_url: "ws://browserless:3333",
  profiles_path: "/profiles",
  connection_timeout_ms: 30000,
  takeover_method: "live_view" as const,
};

describe("BrowserSection connection test", () => {
  beforeEach(() => vi.resetAllMocks());

  it("posts the configured endpoint and reports success", async () => {
    vi.mocked(api.post).mockResolvedValue({ ok: true });
    render(<BrowserSection browser={browser} onChange={vi.fn()} />);
    fireEvent.click(screen.getByRole("button", { name: "Test connection" }));
    await waitFor(() => expect(screen.getByText("Connected")).toBeInTheDocument());
    expect(api.post).toHaveBeenCalledWith("/api/browser/test", {
      ws_url: "ws://browserless:3333",
      profiles_path: "/profiles",
    });
  });

  it("shows why the connection failed", async () => {
    vi.mocked(api.post).mockRejectedValue(new Error("Failed to connect to browserless: refused"));
    render(<BrowserSection browser={browser} onChange={vi.fn()} />);
    fireEvent.click(screen.getByRole("button", { name: "Test connection" }));
    expect(await screen.findByText("Failed to connect to browserless: refused")).toBeInTheDocument();
  });

  it("clears a stale result when the URL changes", async () => {
    vi.mocked(api.post).mockResolvedValue({ ok: true });
    render(<BrowserSection browser={browser} onChange={vi.fn()} />);
    fireEvent.click(screen.getByRole("button", { name: "Test connection" }));
    await screen.findByText("Connected");
    fireEvent.change(screen.getByDisplayValue("ws://browserless:3333"), { target: { value: "ws://other:3333" } });
    expect(screen.queryByText("Connected")).not.toBeInTheDocument();
  });
});
