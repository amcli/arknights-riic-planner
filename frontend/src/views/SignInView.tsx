// Signing in, or creating an account: shown in place of any screen that
// keeps things per account while nobody is signed in. After a session ended
// under an open screen, it says so and offers that account first.

import { useState, type FormEvent } from "react";
import { ApiError, type Account } from "../api";
import { useAuth } from "../auth";
import { Callout, Segmented } from "../components/ui";
import { errorText } from "../format";

type Mode = "sign-in" | "register";

/** The server's own words for what went wrong, without the status code. */
const explain = (err: unknown) => (err instanceof ApiError ? err.detail : errorText(err));

export function SignInView({ ended }: { ended?: Account }) {
  const { signIn, register } = useAuth();
  const [mode, setMode] = useState<Mode>("sign-in");
  const [username, setUsername] = useState(ended?.username ?? "");
  const [password, setPassword] = useState("");
  const [email, setEmail] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const creating = mode === "register";

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    setError(null);
    setBusy(true);
    try {
      if (creating) await register(username.trim(), password, email.trim() || undefined);
      else await signIn(username.trim(), password);
    } catch (err) {
      setError(explain(err));
      setBusy(false);
    }
  };

  return (
    <section className="card auth-card" aria-labelledby="auth-title">
      <header className="card-head">
        <h1 id="auth-title" className="card-title lg">
          {creating ? "Create an account" : "Sign in"}
        </h1>
      </header>
      <form className="card-body stack-sm" onSubmit={(e) => void submit(e)}>
        {ended ? (
          <Callout title="Your session ended">
            Sign in again as {ended.username} to carry on where you left off; anything unsaved is kept.
          </Callout>
        ) : (
          <p className="muted">Rosters, bases and plans are kept per account. Game data is open to everyone.</p>
        )}
        <Segmented
          name="auth-mode"
          label="Sign in or create an account"
          full
          value={mode}
          options={[
            { value: "sign-in", label: "Sign in" },
            { value: "register", label: "Create account" },
          ]}
          onChange={(m) => {
            setMode(m);
            setError(null);
          }}
        />
        <label className="field">
          <span className="field-label">Username</span>
          <input
            type="text"
            value={username}
            onChange={(e) => setUsername(e.target.value)}
            autoComplete="username"
            autoCapitalize="none"
            spellCheck={false}
            required
            minLength={creating ? 3 : undefined}
            maxLength={32}
            pattern={creating ? "[A-Za-z0-9_\\-]+" : undefined}
            title={creating ? "3 to 32 letters, digits, _ or -" : undefined}
          />
        </label>
        <label className="field">
          <span className="field-label">Password</span>
          <input
            type="password"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            autoComplete={creating ? "new-password" : "current-password"}
            required
            minLength={creating ? 8 : undefined}
            maxLength={128}
          />
          {creating && <span className="muted small">At least 8 characters; a passphrase is best.</span>}
        </label>
        {creating && (
          <label className="field">
            <span className="field-label">
              Email <span className="muted">(optional)</span>
            </span>
            <input type="email" value={email} onChange={(e) => setEmail(e.target.value)} autoComplete="email" />
          </label>
        )}
        {error && <Callout tone="error" title={error} />}
        <button type="submit" className="full" disabled={busy}>
          {busy && <span className="spinner" />}
          {creating ? "Create account" : "Sign in"}
        </button>
      </form>
    </section>
  );
}
