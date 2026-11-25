import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ApiError } from "@/lib/api-client";
import { BootLoadError } from "@/lib/navigation-context";
import { AppGate } from "../app-gate";

const refresh = vi.fn();
let boot: Record<string, unknown> = {};

vi.mock("next/navigation", () => ({ useRouter: () => ({ replace: vi.fn() }) }));
// Stable identities, as in the real provider: a fresh `user` per render would
// re-trigger the gate's load effect.
const auth = { connectionState: "online", user: { id: "u1" }, revalidate: vi.fn() };
vi.mock("@/lib/auth", () => ({ useAuth: () => auth }));
vi.mock("@/lib/navigation-context", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/lib/navigation-context")>()),
  useNavigationRaw: () => ({ ...boot, refresh }),
}));

describe("AppGate boot", () => {
  beforeEach(() => {
    vi.resetAllMocks();
    boot = {};
  });

  it("renders the app once every list has loaded", () => {
    boot = { agents: [], spaces: [], tasks: [], contacts: {} };
    refresh.mockResolvedValue(undefined);
    render(<AppGate><p>the app</p></AppGate>);
    expect(screen.getByText("the app")).toBeInTheDocument();
  });

  it("names the list that failed and offers a retry instead of spinning forever", async () => {
    refresh.mockRejectedValueOnce(
      new BootLoadError("chats", new ApiError(500, "Database error: bad row")),
    );
    render(<AppGate><p>the app</p></AppGate>);

    expect(await screen.findByRole("alert")).toHaveTextContent("Couldn't load your chats");
    expect(screen.getByRole("alert")).toHaveTextContent("Database error: bad row");
    expect(screen.queryByText("the app")).not.toBeInTheDocument();

    refresh.mockResolvedValueOnce(undefined);
    fireEvent.click(screen.getByRole("button", { name: "Try again" }));
    await waitFor(() => expect(refresh).toHaveBeenCalledTimes(2));
    await waitFor(() => expect(screen.queryByRole("alert")).not.toBeInTheDocument());
  });

  it("leaves a plain network outage to the reconnect loop", async () => {
    refresh.mockRejectedValueOnce(
      new BootLoadError("tasks", new ApiError(0, "unreachable", "unavailable")),
    );
    render(<AppGate><p>the app</p></AppGate>);
    await waitFor(() => expect(refresh).toHaveBeenCalled());
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });
});
