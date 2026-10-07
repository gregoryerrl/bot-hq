import { render, screen, fireEvent, waitFor, within } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { ModelsPanel, accountLabelOf, limitLine, marksFor } from "./ModelsPanel";
import { invoke } from "@tauri-apps/api/core";
import type { AccountMark, AccountSetupCommands, ModelView } from "../lib/bindings";
import { wideDialogClass } from "../components/ui/Dialog";

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

/** What the backend's `account_setup_commands` returns for a dir — the text
 *  itself is the backend's (agents::account_setup, pinned in Rust); here it is
 *  only something recognisable per dir and shell. */
function setupFor(dir: string, shell: AccountSetupCommands["shell"] = "sh"): AccountSetupCommands {
  return { shell, setup: `SETUP ${shell} ${dir}`, share: `SHARE ${shell} ${dir}` };
}

function mockBackend(
  models: ModelView[],
  marks: AccountMark[] = [],
  shell: AccountSetupCommands["shell"] = "sh",
) {
  mockInvoke.mockImplementation(async (cmd: string, args?: unknown) => {
    switch (cmd) {
      case "list_models":
        return models;
      case "list_account_marks":
        return marks;
      case "account_setup_commands":
        return setupFor((args as { dir: string }).dir, shell);
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

/** `retry` defaults to off here; pass the app's own default (`Providers.tsx`:
 *  1) where a query's own retry setting is what is under test. */
function renderPanel(retry: number | false = false) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry } } });
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

  it("the dialog's dir field shows the backend's setup commands for the dir typed, and sends the dir on save", async () => {
    mockBackend([]);
    renderPanel();
    fireEvent.click(await screen.findByRole("button", { name: /add model/i }));
    const dialog = await screen.findByRole("dialog");
    expect(screen.queryByTestId("account-setup-command")).toBeNull();

    fireEvent.change(screen.getByLabelText(/claude config dir/i), {
      target: { value: "/Users/me/.claude-acct-2" },
    });
    // The backend writes them, in this machine's shell.
    expect(await screen.findByTestId("account-setup-command")).toHaveTextContent(
      "SETUP sh /Users/me/.claude-acct-2",
    );
    expect(screen.getByTestId("account-share-command")).toHaveTextContent("SHARE sh /Users/me/.claude-acct-2");
    expect(mockInvoke).toHaveBeenCalledWith("account_setup_commands", { dir: "/Users/me/.claude-acct-2" });
    expect(dialog.textContent).toContain("in your own terminal");
    expect(dialog.textContent).not.toMatch(/Developer Mode/);
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
    const refusal = await screen.findByText(/must be an absolute path/i);
    // Beside Save, outside the scrolling body: a refusal below the fold would
    // leave Save looking dead.
    expect(screen.getByTestId("model-dialog-body")).not.toContainElement(refusal);
  });
});

describe("Settings → Models — the dialog fits the window", () => {
  // jsdom has no layout, so this pins the classes that make it fit: a frame
  // with a fixed, viewport-clamped height (the New session frame), a body that
  // can shrink (min-h-0) and scrolls, and actions outside the scroller.
  // Before this, the dialog was a centred card with no height bound — taller
  // than the window once the config dir's setup commands showed, with Save
  // pushed off-screen and nothing to scroll.
  const tokens = (el: Element) => el.className.split(/\s+/);

  it("is the New session frame, its fields scroll, and Cancel/Save never do", async () => {
    mockBackend([]);
    renderPanel();
    fireEvent.click(await screen.findByRole("button", { name: /add model/i }));
    const dialog = await screen.findByRole("dialog", { name: /add model/i });
    expect(dialog.className).toBe(wideDialogClass);
    expect(tokens(dialog)).toEqual(
      expect.arrayContaining(["flex", "flex-col", "overflow-hidden", "h-[min(760px,90vh)]"]),
    );

    // Below md the body is the one scroller; on md+ each column scrolls itself.
    const body = within(dialog).getByTestId("model-dialog-body");
    expect(tokens(body)).toEqual(expect.arrayContaining(["min-h-0", "flex-1", "max-md:overflow-y-auto"]));
    const columns = Array.from(body.children);
    expect(columns).toHaveLength(2);
    for (const column of columns) {
      expect(tokens(column)).toEqual(expect.arrayContaining(["min-h-0", "md:overflow-y-auto"]));
    }

    for (const label of [/display name/i, /^model id$/i, /context window/i, /claude config dir/i, /claude cli settings/i]) {
      expect(body).toContainElement(within(dialog).getByLabelText(label));
    }
    expect(body).not.toContainElement(within(dialog).getByRole("button", { name: /^save$/i }));
    expect(body).not.toContainElement(within(dialog).getByRole("button", { name: /^cancel$/i }));
  });

  it("a selection dragged out of the dialog leaves it open; the backdrop and × close it", async () => {
    mockBackend([]);
    renderPanel();
    const open = async () => {
      fireEvent.click(await screen.findByRole("button", { name: /add model/i }));
      return screen.findByRole("dialog", { name: /add model/i });
    };
    const dialog = await open();
    // A drag that starts inside the dialog and is released over the backdrop
    // sends its click to the nearest element holding both — the dialog's
    // parent. When that parent was a closing overlay, the drag closed the
    // dialog and dropped the draft.
    fireEvent.click(dialog.parentElement!);
    expect(screen.getByRole("dialog", { name: /add model/i })).toBeInTheDocument();

    const backdrop = dialog.previousElementSibling!;
    expect(backdrop).toHaveAttribute("aria-hidden");
    fireEvent.click(backdrop);
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());

    const reopened = await open();
    fireEvent.click(within(reopened).getByRole("button", { name: /^close$/i }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
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

});

describe("Settings → Models — the second account's setup commands", () => {
  async function openWithDir(dir: string, retry: number | false = false) {
    renderPanel(retry);
    fireEvent.click(await screen.findByRole("button", { name: /add model/i }));
    const dialog = await screen.findByRole("dialog");
    fireEvent.change(screen.getByLabelText(/claude config dir/i), { target: { value: dir } });
    return dialog;
  }

  it("on Windows names PowerShell and what its links need", async () => {
    mockBackend([], [], "powershell");
    const dialog = await openWithDir("C:\\Users\\me\\.claude-acct-2");
    expect(await screen.findByTestId("account-setup-command")).toHaveTextContent(
      "SETUP powershell C:\\Users\\me\\.claude-acct-2",
    );
    expect(dialog.textContent).toContain("in PowerShell");
    expect(dialog.textContent).not.toContain("in your own terminal");
    expect(dialog.textContent).toMatch(/junctions; CLAUDE\.md and settings\.json\s+need Developer Mode/);
  });

  it("shows the backend's refusal instead of a command for a dir Save would refuse", async () => {
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "list_models") return [];
      if (cmd === "account_setup_commands") {
        throw { kind: "validation", message: "Claude config dir must be an absolute path — the CLI does not expand `~`" };
      }
      return undefined;
    });
    // Under the app's default retry (1): the refusal still shows at once.
    await openWithDir("~/.claude-acct-2", 1);
    expect(await screen.findByTestId("account-setup-refusal")).toHaveTextContent("does not expand `~`");
    expect(screen.queryByTestId("account-setup-command")).toBeNull();
    // Asked once: a refusal is final, not retried after a back-off.
    expect(mockInvoke.mock.calls.filter((c) => c[0] === "account_setup_commands")).toHaveLength(1);
  });

  it("hides the commands once a token is typed or the dir is cleared, though the last ones are still cached", async () => {
    mockBackend([]);
    await openWithDir("/Users/me/.claude-acct-2");
    expect(await screen.findByTestId("account-setup-command")).toBeInTheDocument();

    fireEvent.change(screen.getByLabelText(/auth token/i), { target: { value: "sk-gateway" } });
    expect(screen.queryByTestId("account-setup-command")).toBeNull();
    fireEvent.change(screen.getByLabelText(/auth token/i), { target: { value: "" } });
    expect(await screen.findByTestId("account-setup-command")).toBeInTheDocument();

    fireEvent.change(screen.getByLabelText(/claude config dir/i), { target: { value: "" } });
    expect(screen.queryByTestId("account-setup-command")).toBeNull();
  });
});
