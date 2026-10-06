import { render, screen, fireEvent, waitFor, within } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  ModelsPanel,
  accountLabelOf,
  accountSetupCommand,
  accountShareCommand,
  limitLine,
  marksFor,
} from "./ModelsPanel";
import { invoke } from "@tauri-apps/api/core";
import type { AccountMark, ModelView } from "../lib/bindings";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
const mockInvoke = vi.mocked(invoke);

const OPUS: ModelView = {
  id: "m-opus",
  display_name: "Opus",
  provider: "anthropic",
  model_name: "claude-opus-5-5",
  base_url: null,
  auth_token: null,
  created_at: "2026-10-06T00:00:00Z",
  updated_at: "2026-10-06T00:00:00Z",
  context_window: null,
  cli_settings: null,
  claude_config_dir: null,
};

function mockBackend(models: ModelView[], marks: AccountMark[] = []) {
  mockInvoke.mockImplementation(async (cmd: string) => {
    switch (cmd) {
      case "list_models":
        return models;
      case "list_account_marks":
        return marks;
      case "clear_account_mark":
        return true;
      case "upsert_model":
      case "delete_model":
        return undefined;
      default:
        return undefined;
    }
  });
}

function renderPanel() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={qc}>
      <ModelsPanel />
    </QueryClientProvider>,
  );
}

beforeEach(() => mockInvoke.mockReset());

describe("Settings → Models — a model row's Claude config dir (0090)", () => {
  it("lists the account a subscription row bills, and nothing for the default dir or a gateway", async () => {
    mockBackend([
      OPUS,
      { ...OPUS, id: "m-opus-b", display_name: "Opus · acct 2", claude_config_dir: "/Users/me/.claude-acct-2" },
      {
        ...OPUS,
        id: "m-ds",
        display_name: "DeepSeek",
        provider: "deepseek",
        model_name: "deepseek-v4-pro",
        base_url: "https://api.deepseek.com/anthropic",
        auth_token: "t",
        // A dir on a gateway row is ignored for billing and not shown.
        claude_config_dir: "/Users/me/.claude-acct-2",
      },
    ]);
    renderPanel();
    await screen.findByText("Opus · acct 2");
    const chips = screen.getAllByTestId("model-account");
    expect(chips).toHaveLength(1);
    expect(chips[0]).toHaveTextContent(".claude-acct-2");
    expect(chips[0]).toHaveAttribute("title", "Claude config dir: /Users/me/.claude-acct-2");
  });

  it("the dialog's dir field shows the one-time login command for the dir typed, and sends the dir on save", async () => {
    mockBackend([]);
    renderPanel();
    fireEvent.click(await screen.findByRole("button", { name: /add model/i }));
    const dialog = await screen.findByRole("dialog");
    expect(screen.queryByTestId("account-setup-command")).toBeNull();

    fireEvent.change(screen.getByLabelText(/claude config dir/i), {
      target: { value: "/Users/me/.claude-acct-2" },
    });
    expect(screen.getByTestId("account-setup-command")).toHaveTextContent(
      "mkdir -p '/Users/me/.claude-acct-2' && CLAUDE_CONFIG_DIR='/Users/me/.claude-acct-2' claude auth login --claudeai",
    );
    // The subscription sign-in, never the interactive menu (which offers
    // Console = API billing), and nothing bot-hq runs itself.
    expect(dialog.textContent).not.toMatch(/\/login\b/);

    fireEvent.change(screen.getByLabelText(/display name/i), { target: { value: "Opus · acct 2" } });
    fireEvent.change(screen.getByLabelText(/^model id$/i), { target: { value: "claude-opus-5-5" } });
    fireEvent.click(screen.getByRole("button", { name: /^save$/i }));
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith(
        "upsert_model",
        expect.objectContaining({
          model: expect.objectContaining({ claude_config_dir: "/Users/me/.claude-acct-2" }),
        }),
      ),
    );
  });

  it("shows the backend's refusal of a bad dir inline", async () => {
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "list_models") return [];
      if (cmd === "upsert_model") {
        throw { kind: "validation", message: "Claude config dir must be an absolute path — the CLI does not expand `~`" };
      }
      return undefined;
    });
    renderPanel();
    fireEvent.click(await screen.findByRole("button", { name: /add model/i }));
    await screen.findByRole("dialog");
    fireEvent.change(screen.getByLabelText(/display name/i), { target: { value: "x" } });
    fireEvent.change(screen.getByLabelText(/^model id$/i), { target: { value: "y" } });
    fireEvent.change(screen.getByLabelText(/claude config dir/i), { target: { value: "~/.claude-acct-2" } });
    fireEvent.click(screen.getByRole("button", { name: /^save$/i }));
    expect(await screen.findByText(/must be an absolute path/i)).toBeInTheDocument();
  });
});

describe("Settings → Models — usage-limit marks (0091)", () => {
  it("shows a marked account's limit on its rows with a Clear, and nothing on a gateway row", async () => {
    const until = new Date(Date.now() + 3600 * 1000).toISOString();
    mockBackend(
      [
        OPUS,
        { ...OPUS, id: "m-opus-b", display_name: "Opus · acct 2", claude_config_dir: "/Users/me/.claude-acct-2" },
        { ...OPUS, id: "m-fable-b", display_name: "Fable · acct 2", model_name: "claude-fable-5-1", claude_config_dir: "/Users/me/.claude-acct-2" },
        { ...OPUS, id: "m-ds", display_name: "DeepSeek", provider: "deepseek", auth_token: "t", base_url: "https://x", claude_config_dir: "/Users/me/.claude-acct-2" },
      ],
      [
        {
          config_dir: "/Users/me/.claude-acct-2",
          org_id: "org-2",
          model_name: "claude-opus-5-5",
          email: "two@example.com",
          limited_until: until,
          limited_text: "You've hit your session limit · resets 8pm (Asia/Manila)",
          marked_at: new Date().toISOString(),
        },
      ],
    );
    renderPanel();
    await screen.findByText("Opus · acct 2");
    // Opus on acct 2 is marked; Fable on the same account is not (per model),
    // and the gateway row never is.
    const limits = await screen.findAllByTestId("model-limit");
    expect(limits).toHaveLength(1);
    expect(limits[0].textContent).toContain("two@example.com: limited until about");
    expect(limits[0].textContent).toContain("hit your session limit");
    expect(limits[0].textContent).not.toMatch(/switch|other account/i);
    fireEvent.click(within(limits[0]).getByRole("button", { name: /clear/i }));
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("clear_account_mark", {
        configDir: "/Users/me/.claude-acct-2",
        orgId: "org-2",
        modelName: "claude-opus-5-5",
      }),
    );
  });

  it("marksFor and limitLine", () => {
    const mark: AccountMark = {
      config_dir: "",
      org_id: "org-1",
      model_name: "claude-opus-5-5",
      email: null,
      limited_until: null,
      limited_text: "You're out of usage credits.",
      marked_at: "2026-10-06T13:45:00Z",
    };
    expect(marksFor(OPUS, [mark])).toEqual([mark]);
    expect(marksFor({ ...OPUS, claude_config_dir: "/x" }, [mark])).toEqual([]);
    expect(marksFor({ ...OPUS, model_name: "claude-fable-5-1" }, [mark])).toEqual([]);
    expect(marksFor({ ...OPUS, auth_token: "t" }, [mark])).toEqual([]);
    expect(limitLine(mark)).toBe('limited (no reset time given) — "You\'re out of usage credits."');
  });
});

describe("account helpers", () => {
  it("accountLabelOf: the dir's last segment, default, or the provider for a gateway row", () => {
    expect(accountLabelOf(OPUS)).toBe("default");
    expect(accountLabelOf({ ...OPUS, claude_config_dir: "/Users/me/.claude-acct-2/" })).toBe(".claude-acct-2");
    expect(accountLabelOf({ ...OPUS, provider: "deepseek", auth_token: "t", claude_config_dir: "/x" })).toBe("deepseek");
    expect(accountLabelOf({ ...OPUS, provider: "OpenRouter", base_url: "https://openrouter.ai/api" })).toBe("OpenRouter");
  });

  it("the setup commands quote the dir and never share projects/", () => {
    expect(accountSetupCommand("/Users/me/.claude-acct-2")).toBe(
      "mkdir -p '/Users/me/.claude-acct-2' && CLAUDE_CONFIG_DIR='/Users/me/.claude-acct-2' claude auth login --claudeai",
    );
    const share = accountShareCommand("/Users/me/.claude-acct-2");
    expect(share).toContain("B='/Users/me/.claude-acct-2'");
    expect(share).toContain("for item in CLAUDE.md settings.json agents commands skills plugins; do");
    expect(share).not.toMatch(/projects/);
  });
});
