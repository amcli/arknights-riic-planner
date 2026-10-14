// Who is signed in, for the whole app. The session lives in an HttpOnly
// cookie the page cannot read, so the page asks the server on load, after
// a sign-in or sign-out in another tab (they share the cookie), and goes
// back to signed out whenever the server refuses a request for want of a
// session.

import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { api, ApiError, onSignedOut, type Account } from "./api";
import { errorText } from "./format";
import { rememberAccount } from "./storage";

export type AuthState =
  | { status: "loading" }
  /**
   * `ended` is the account whose session ended under it (it expired, or
   * was ended elsewhere) rather than by signing out here: its screens are
   * kept, unsaved edits and all, for it to sign back in to.
   */
  | { status: "signed-out"; ended?: Account }
  | { status: "signed-in"; account: Account }
  /** The server could not say; nothing that needs an account can run. */
  | { status: "unknown"; error: string };

export interface Auth {
  state: AuthState;
  signIn: (username: string, password: string) => Promise<void>;
  register: (username: string, password: string, email?: string) => Promise<void>;
  signOut: () => Promise<void>;
}

/** Tabs tell each other here when they sign in or out. */
const CHANNEL = "riic-planner:auth";

const Context = createContext<Auth | null>(null);

export function AuthProvider({ children }: { children: ReactNode }) {
  const [state, setState] = useState<AuthState>({ status: "loading" });
  const channel = useRef<BroadcastChannel | null>(null);

  const adopt = useCallback((account: Account) => {
    rememberAccount(account.id);
    setState({ status: "signed-in", account });
  }, []);

  // Asks the server who is signed in. Nobody is a plain signed-out: this is
  // not a session ending under an open screen.
  const check = useCallback(async () => {
    try {
      adopt(await api.auth.me());
    } catch (err) {
      if (err instanceof ApiError && err.status === 401) setState({ status: "signed-out" });
      else setState({ status: "unknown", error: errorText(err) });
    }
  }, [adopt]);

  useEffect(() => {
    void check();
    const unsubscribe = onSignedOut(() =>
      setState((s) =>
        s.status === "signed-in" ? { status: "signed-out", ended: s.account } : s.status === "signed-out" ? s : { status: "signed-out" },
      ),
    );
    const tabs = typeof BroadcastChannel === "function" ? new BroadcastChannel(CHANNEL) : null;
    if (tabs) tabs.onmessage = () => void check();
    channel.current = tabs;
    return () => {
      unsubscribe();
      tabs?.close();
      channel.current = null;
    };
  }, [check]);

  const auth = useMemo<Auth>(() => {
    const announce = () => channel.current?.postMessage("changed");
    return {
      state,
      signIn: async (username, password) => {
        adopt(await api.auth.login(username, password));
        announce();
      },
      register: async (username, password, email) => {
        adopt(await api.auth.register(username, password, email));
        announce();
      },
      signOut: async () => {
        await api.auth.logout();
        setState({ status: "signed-out" });
        announce();
      },
    };
  }, [state, adopt]);
  return <Context.Provider value={auth}>{children}</Context.Provider>;
}

export function useAuth(): Auth {
  const auth = useContext(Context);
  if (!auth) throw new Error("useAuth outside AuthProvider");
  return auth;
}
