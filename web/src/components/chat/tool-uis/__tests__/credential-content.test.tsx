import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { api } from "@/lib/api-client";
import type { ToolCall } from "@/lib/types";
import { CredentialContent, CredentialsContent } from "../tool-content";

vi.mock("@/lib/api-client", () => ({ api: { get: vi.fn() } }));

const te = {
  hitl: {
    prompt: "Database access", url: "", status: "pending", response: null, delivery: null,
    request: { type: "Credential", data: { query: "database", reason: "Connect to the database" } },
  },
} as ToolCall;

describe("chat credential picker", () => {
  beforeEach(() => {
    vi.resetAllMocks();
    vi.mocked(api.get).mockImplementation(async (path) => {
      if (path === "/api/vaults") return [
        { id: "local", name: "Local vault", provider: "local", enabled: true },
        { id: "work", name: "Work vault", provider: "bitwarden", enabled: true },
      ];
      if (path.endsWith("/fields")) return ["USERNAME", "PASSWORD", "API_KEY"];
      return [{ id: "same-id", name: "Database", username: null }];
    });
  });

  it("searches all vaults using the requested query and approves the chosen source", async () => {
    const onResolve = vi.fn();
    render(<CredentialContent te={te} chatId="chat-1" onResolve={onResolve} />);
    expect(await screen.findByRole("searchbox")).toHaveValue("database");
    expect(await screen.findByRole("button", { name: "Database, Local vault" })).toBeInTheDocument();
    expect(api.get).toHaveBeenCalledWith("/api/vaults/local/items?q=database");
    expect(api.get).toHaveBeenCalledWith("/api/vaults/work/items?q=database");
    expect(screen.getByRole("button", { name: "Approve" })).toBeDisabled();
    expect(screen.queryByRole("button", { name: "Entire credential" })).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Database, Work vault" }));
    expect(await screen.findByText("DATABASE_PASSWORD")).toBeInTheDocument();
    expect(screen.getByText("Advanced").closest("details")).not.toHaveAttribute("open");
    fireEvent.change(screen.getByRole("combobox"), { target: { value: "hours" } });
    fireEvent.click(screen.getByRole("button", { name: "Approve" }));
    expect(onResolve).toHaveBeenCalledWith({
      type: "Vault", data: { type: "Granted", data: {
        connection_id: "work", vault_item_id: "same-id", grant_duration: { hours: 24 },
        target: { Prefix: { env_var_prefix: "DATABASE" } },
      } },
    }, "Approved");

    fireEvent.change(screen.getByRole("searchbox"), { target: { value: "" } });
    expect(screen.getByRole("button", { name: "Approve" })).toBeDisabled();
    expect(await screen.findByRole("button", { name: "Database, Work vault" })).toBeInTheDocument();
    expect(api.get).toHaveBeenCalledWith("/api/vaults/work/items?q=");
  });

  it("previews and approves an actual field with a default or customized variable name", async () => {
    const onResolve = vi.fn();
    render(<CredentialContent te={te} chatId="chat-1" onResolve={onResolve} />);
    fireEvent.click(await screen.findByRole("button", { name: "Database, Work vault" }));
    await screen.findByText("DATABASE_PASSWORD");
    fireEvent.click(screen.getByRole("button", { name: "A specific field" }));
    expect(screen.getByLabelText("Field to share")).toHaveValue("PASSWORD");
    expect(screen.queryByText("DATABASE_USERNAME")).not.toBeInTheDocument();
    fireEvent.change(screen.getByLabelText("Field to share"), { target: { value: "API_KEY" } });
    expect(screen.getByText("DATABASE_API_KEY")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Approve" }));
    expect(onResolve).toHaveBeenLastCalledWith(expect.objectContaining({
      data: { type: "Granted", data: {
        connection_id: "work", vault_item_id: "same-id", grant_duration: "once",
        target: { Single: { env_var: "DATABASE_API_KEY", field: { Custom: { name: "API_KEY" } } } },
      } },
    }), "Approved");

    fireEvent.click(screen.getByText("Advanced"));
    fireEvent.change(screen.getByLabelText("Environment variable name"), { target: { value: "SERVICE_TOKEN" } });
    expect(screen.getByText("SERVICE_TOKEN")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Approve" }));
    expect(onResolve).toHaveBeenLastCalledWith(expect.objectContaining({
      data: { type: "Granted", data: {
        connection_id: "work", vault_item_id: "same-id", grant_duration: "once",
        target: { Single: { env_var: "SERVICE_TOKEN", field: { Custom: { name: "API_KEY" } } } },
      } },
    }), "Approved");
  });

  it("blocks approval when fields fail to load and allows retry", async () => {
    const onResolve = vi.fn();
    render(<CredentialContent te={te} chatId="chat-1" onResolve={onResolve} />);
    const item = await screen.findByRole("button", { name: "Database, Work vault" });
    vi.mocked(api.get).mockRejectedValueOnce(new Error("Unavailable"));
    fireEvent.click(item);
    expect(await screen.findByRole("alert")).toHaveTextContent("Could not load credential fields");
    expect(screen.getByRole("button", { name: "Approve" })).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    await waitFor(() => expect(screen.getByRole("button", { name: "Approve" })).toBeEnabled());
    expect(screen.getByText("DATABASE_PASSWORD")).toBeInTheDocument();
  });

  it("allows retrying connection failures and declining without a selection", async () => {
    vi.mocked(api.get).mockRejectedValueOnce(new Error("Unavailable"));
    const onResolve = vi.fn();
    render(<CredentialContent te={te} chatId="chat-1" onResolve={onResolve} />);
    expect(await screen.findByRole("alert")).toHaveTextContent("Could not load vaults");
    expect(screen.getByRole("button", { name: "Approve" })).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    expect(await screen.findByRole("button", { name: "Database, Work vault" })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Decline" }));
    expect(onResolve).toHaveBeenCalledWith({ type: "Vault", data: { type: "Denied" } }, "Denied");
  });
});

describe("batched credential request", () => {
  const batch = {
    hitl: {
      prompt: "Two keys", url: "", status: "pending", response: null, delivery: null,
      request: { type: "Credentials", data: {
        reason: "Call the weather API",
        items: [{ query: "app key", label: "App key" }, { query: "user key", label: "User key" }],
      } },
    },
  } as ToolCall;

  beforeEach(() => {
    vi.resetAllMocks();
    vi.mocked(api.get).mockImplementation(async (path) => {
      if (path === "/api/vaults") return [{ id: "local", name: "Local vault", provider: "local", enabled: true }];
      if (path.endsWith("/fields")) return ["PASSWORD"];
      return [{ id: "item-1", name: "Key", username: null }];
    });
  });

  it("grants one slot, skips the other with a reason, and sends both", async () => {
    const onResolve = vi.fn();
    render(<CredentialsContent te={batch} chatId="chat-1" onResolve={onResolve} />);
    expect(screen.getByRole("button", { name: "Approve" })).toBeDisabled();

    const appKey = within(screen.getByRole("region", { name: "App key" }));
    fireEvent.click(await appKey.findByRole("button", { name: "Key, Local vault" }));
    expect(screen.getByRole("button", { name: "Approve" })).toBeDisabled();

    fireEvent.click(screen.getByRole("button", { name: "Skip User key" }));
    fireEvent.change(screen.getByLabelText("Reason for skipping User key"), { target: { value: "I don't have one yet" } });
    await waitFor(() => expect(screen.getByRole("button", { name: "Approve" })).toBeEnabled());
    fireEvent.click(screen.getByRole("button", { name: "Approve" }));

    expect(onResolve).toHaveBeenCalledWith({
      type: "Vault", data: { type: "GrantedMany", data: {
        grants: [{
          query: "app key", connection_id: "local", vault_item_id: "item-1", grant_duration: "once",
          target: { Prefix: { env_var_prefix: "APP_KEY" } },
        }],
        skipped: [{ query: "user key", reason: "I don't have one yet" }],
      } },
    }, "Approved 1, skipped 1");
  });

  it("allows skipping every slot, and a skip without a reason omits it", async () => {
    const onResolve = vi.fn();
    render(<CredentialsContent te={batch} chatId="chat-1" onResolve={onResolve} />);
    fireEvent.click(screen.getByRole("button", { name: "Skip App key" }));
    fireEvent.click(screen.getByRole("button", { name: "Skip User key" }));
    fireEvent.click(screen.getByRole("button", { name: "Approve" }));
    expect(onResolve).toHaveBeenCalledWith({
      type: "Vault", data: { type: "GrantedMany", data: {
        grants: [], skipped: [{ query: "app key" }, { query: "user key" }],
      } },
    }, "Skipped all");
  });

  it("lets a skipped slot be provided again", async () => {
    render(<CredentialsContent te={batch} chatId="chat-1" onResolve={vi.fn()} />);
    fireEvent.click(screen.getByRole("button", { name: "Skip App key" }));
    fireEvent.click(screen.getByRole("button", { name: "Provide App key" }));
    expect(await screen.findAllByRole("searchbox")).toHaveLength(2);
  });
});
