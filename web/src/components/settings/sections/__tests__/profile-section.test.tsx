import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  get: vi.fn(),
  put: vi.fn(),
  revalidate: vi.fn(),
  changePassword: vi.fn(),
  authConfig: { sso: { disable_local_auth: false } } as {
    sso: { disable_local_auth: boolean };
  },
}));

vi.mock("@/lib/api-client", () => ({ api: { get: mocks.get, put: mocks.put } }));
vi.mock("@/lib/theme", () => ({ useTheme: () => ({ mode: "system", setMode: vi.fn() }) }));
vi.mock("@/lib/auth", () => ({
  useAuth: () => ({
    user: {
      id: "u1",
      name: "Ada",
      email: "ada@example.com",
      phone: null,
      timezone: "Europe/London",
    },
    revalidate: mocks.revalidate,
    authConfig: mocks.authConfig,
    changePassword: mocks.changePassword,
  }),
}));

import { ProfileSection } from "../profile-section";

describe("ProfileSection", () => {
  beforeEach(() => {
    mocks.get.mockReset().mockResolvedValue([]);
    mocks.put.mockReset().mockResolvedValue({});
    mocks.revalidate.mockReset().mockResolvedValue(undefined);
    mocks.changePassword.mockReset().mockResolvedValue(undefined);
    mocks.authConfig.sso.disable_local_auth = false;
  });

  it("saves a changed name with the full profile so other fields are not cleared", async () => {
    render(<ProfileSection />);
    const input = screen.getByPlaceholderText("Your name");
    fireEvent.change(input, { target: { value: "  Ada L  " } });
    fireEvent.blur(input);

    await waitFor(() => expect(mocks.put).toHaveBeenCalledOnce());
    expect(mocks.put).toHaveBeenCalledWith("/api/auth/profile", {
      timezone: "Europe/London",
      phone: null,
      email: "ada@example.com",
      name: "Ada L",
    });
    expect(mocks.revalidate).toHaveBeenCalled();
  });

  it("does not call the API when the email is left unchanged", () => {
    render(<ProfileSection />);
    fireEvent.blur(screen.getByPlaceholderText("you@example.com"));
    expect(mocks.put).not.toHaveBeenCalled();
  });

  it("shows the server error and restores the email when it is rejected", async () => {
    mocks.put.mockRejectedValueOnce(new Error("Email already in use."));
    render(<ProfileSection />);
    const input = screen.getByPlaceholderText("you@example.com");
    fireEvent.change(input, { target: { value: "taken@example.com" } });
    fireEvent.blur(input);

    expect(await screen.findByText("Email already in use.")).toBeInTheDocument();
    expect(input).toHaveValue("ada@example.com");
  });

  it("refuses mismatched new passwords without calling the API", () => {
    render(<ProfileSection />);
    const [current, next, confirm] = Array.from(
      document.querySelectorAll<HTMLInputElement>('input[type="password"]'),
    );
    fireEvent.change(current, { target: { value: "old-password" } });
    fireEvent.change(next, { target: { value: "new-password-1" } });
    fireEvent.change(confirm, { target: { value: "new-password-2" } });
    fireEvent.click(screen.getByRole("button", { name: "Change password" }));

    expect(screen.getByText("New passwords don't match.")).toBeInTheDocument();
    expect(mocks.changePassword).not.toHaveBeenCalled();
  });

  it("changes the password and clears the fields", async () => {
    render(<ProfileSection />);
    const [current, next, confirm] = Array.from(
      document.querySelectorAll<HTMLInputElement>('input[type="password"]'),
    );
    fireEvent.change(current, { target: { value: "old-password" } });
    fireEvent.change(next, { target: { value: "new-password-1" } });
    fireEvent.change(confirm, { target: { value: "new-password-1" } });
    fireEvent.click(screen.getByRole("button", { name: "Change password" }));

    await waitFor(() =>
      expect(mocks.changePassword).toHaveBeenCalledWith("old-password", "new-password-1"),
    );
    expect(await screen.findByText(/Password changed/)).toBeInTheDocument();
    expect(current).toHaveValue("");
  });

  it("hides the password panel when local auth is disabled by SSO", () => {
    mocks.authConfig.sso.disable_local_auth = true;
    render(<ProfileSection />);
    expect(screen.queryByRole("button", { name: "Change password" })).toBeNull();
  });
});
