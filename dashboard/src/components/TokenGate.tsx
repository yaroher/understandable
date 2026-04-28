import { useEffect, useState } from "react";
import type { ReactNode } from "react";

/**
 * `TokenGate` — placeholder companion for the upcoming chat / explain
 * features.
 *
 * In the original TypeScript Understand-Anything plugin, `TokenGate`
 * blocked the dashboard until the user pasted the access token printed
 * by the dev server (the JSON files were served behind that token). The
 * Rust `understandable` server replaces that model entirely:
 *
 *   - Static graph data is served by the local server bound to
 *     `127.0.0.1` only — no token is needed for read access.
 *   - The chat / explain UI that *will* require an Anthropic API key has
 *     not landed in this Rust port yet (no `/api/chat` route exists in
 *     `crates/ua-server/src/routes/api.rs`).
 *
 * To preserve the upstream component contract while we work on those
 * features, `TokenGate` currently behaves as a transparent pass-through
 * wrapper. When chat / explain ship, this component should:
 *
 *   1. Render an input form to collect the Anthropic API key.
 *   2. Persist the key to `sessionStorage` (never `localStorage`).
 *   3. Wrap chat / explain components and skip rendering them until a
 *      key is available.
 *
 * For non-AI features (graph rendering, search, navigation, tours) the
 * gate is intentionally a no-op so the dashboard works fully offline.
 *
 * TODO: wire to chat / explain UI when those features land in the
 * dashboard. See:
 *   - upstream reference:
 *     `Understand-Anything/understand-anything-plugin/packages/dashboard/src/components/TokenGate.tsx`
 *   - server route inventory:
 *     `crates/ua-server/src/routes/api.rs`
 */
export interface TokenGateProps {
  /** The protected children that should only render once a token is available. */
  children: ReactNode;
  /**
   * If `true`, the gate insists on a token before rendering children.
   * Defaults to `false` because no chat / explain UI ships in this
   * build yet.
   */
  required?: boolean;
  /**
   * Called once a valid-looking token has been entered. Useful when the
   * caller wants to wire the token into a chat client.
   */
  onTokenValid?: (token: string) => void;
}

const TOKEN_STORAGE_KEY = "ua.anthropic.apiKey";

function readStoredToken(): string | null {
  if (typeof window === "undefined") return null;
  try {
    return window.sessionStorage.getItem(TOKEN_STORAGE_KEY);
  } catch {
    return null;
  }
}

function writeStoredToken(value: string): void {
  if (typeof window === "undefined") return;
  try {
    window.sessionStorage.setItem(TOKEN_STORAGE_KEY, value);
  } catch {
    // Storage might be disabled (private mode etc.) — ignore.
  }
}

export default function TokenGate({
  children,
  required = false,
  onTokenValid,
}: TokenGateProps) {
  const [token, setToken] = useState<string | null>(() => readStoredToken());
  const [input, setInput] = useState("");
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (token && onTokenValid) onTokenValid(token);
  }, [token, onTokenValid]);

  // No-op pass-through when the gate is not required.
  if (!required || token) {
    return <>{children}</>;
  }

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    const trimmed = input.trim();
    if (!trimmed) {
      setError("Please paste an API key.");
      return;
    }
    if (!trimmed.startsWith("sk-")) {
      setError("That doesn't look like an Anthropic API key (expected `sk-...`).");
      return;
    }
    writeStoredToken(trimmed);
    setToken(trimmed);
  };

  return (
    <div className="h-screen w-screen flex items-center justify-center bg-root noise-overlay">
      <div className="w-full max-w-md px-8 py-10 bg-surface border border-border-subtle rounded-lg shadow-2xl">
        <h1 className="font-serif text-2xl text-text-primary tracking-wide text-center mb-2">
          Anthropic API key required
        </h1>
        <p className="text-text-muted text-sm text-center mb-8">
          Paste your <code className="font-mono">sk-...</code> key to enable
          chat &amp; explain features. The key is kept in this tab&apos;s
          session storage only and never sent to anyone but Anthropic.
        </p>

        <form onSubmit={handleSubmit} className="flex flex-col gap-4">
          <input
            type="password"
            value={input}
            onChange={(e) => {
              setInput(e.target.value);
              if (error) setError(null);
            }}
            placeholder="sk-ant-..."
            autoFocus
            className="w-full px-4 py-3 bg-elevated border border-border-subtle rounded text-text-primary placeholder:text-text-muted/50 font-mono text-sm focus:outline-none focus:border-accent transition-colors"
          />

          {error && <p className="text-red-400 text-sm">{error}</p>}

          <button
            type="submit"
            disabled={!input.trim()}
            className="w-full py-3 bg-accent text-root font-semibold rounded transition-all hover:brightness-110 disabled:opacity-40 disabled:cursor-not-allowed"
          >
            Continue
          </button>
        </form>
      </div>
    </div>
  );
}
