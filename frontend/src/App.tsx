// The app shell: a bar with one tab per screen, the account and the theme
// switch, and the screen the hash names. Game data loads once for all of
// them and is open to everyone; the screens that keep things per account
// ask for a sign-in instead while nobody is signed in. Each screen draws
// its own header.

import { Fragment, useEffect, useState, type ComponentType } from "react";
import { AuthProvider, useAuth } from "./auth";
import {
  BrandMark,
  IconBase,
  IconData,
  IconMonitor,
  IconMoon,
  IconPlan,
  IconResults,
  IconRoster,
  IconSun,
  type IconProps,
} from "./components/icons";
import { GameDataProvider } from "./data";
import { Callout } from "./components/ui";
import { errorText } from "./format";
import { href, PAGES, useRoute, type Page, type Route } from "./router";
import { recall, remember } from "./storage";
import { BaseView } from "./views/BaseView";
import { DataView } from "./views/DataView";
import { PlanView } from "./views/PlanView";
import { ResultsView } from "./views/ResultsView";
import { RostersView } from "./views/RostersView";
import { SignInView } from "./views/SignInView";

const ICONS: Record<Page, ComponentType<IconProps>> = {
  rosters: IconRoster,
  base: IconBase,
  plan: IconPlan,
  results: IconResults,
  data: IconData,
};

type Theme = "system" | "light" | "dark";
const NEXT_THEME: Record<Theme, Theme> = { system: "light", light: "dark", dark: "system" };

/** The theme picked in this browser; `index.html` applies it before first paint. */
function useTheme(): [Theme, () => void] {
  const [theme, setTheme] = useState<Theme>(() => {
    const saved = recall("theme");
    return saved === "light" || saved === "dark" ? saved : "system";
  });
  useEffect(() => {
    const root = document.documentElement;
    if (theme === "system") delete root.dataset.theme;
    else root.dataset.theme = theme;
    remember("theme", theme === "system" ? null : theme);
  }, [theme]);
  return [theme, () => setTheme((t) => NEXT_THEME[t])];
}

export function App() {
  return (
    <AuthProvider>
      <Shell />
    </AuthProvider>
  );
}

function Shell() {
  const route = useRoute();
  const { state } = useAuth();
  // The account controls start afresh for each account, so no stale error
  // carries over.
  const accountKey = state.status === "signed-in" ? state.account.id : state.status;
  const [theme, cycleTheme] = useTheme();
  const ThemeIcon = theme === "light" ? IconSun : theme === "dark" ? IconMoon : IconMonitor;
  return (
    <>
      <header className="app-bar">
        <div className="app-bar-inner">
          <a className="brand" href={href("rosters")}>
            <BrandMark />
            <span className="brand-name">RIIC Planner</span>
          </a>
          <nav className="nav" aria-label="Screens">
            {PAGES.map((p) => {
              const Icon = ICONS[p.page];
              return (
                <a key={p.page} href={href(p.page)} aria-current={route.page === p.page ? "page" : undefined}>
                  <Icon />
                  <span>{p.label}</span>
                </a>
              );
            })}
          </nav>
          <div className="bar-actions">
            <AccountActions key={accountKey} />
            <button
              type="button"
              className="ghost icon"
              onClick={cycleTheme}
              aria-label={`Theme: ${theme}. Switch to ${NEXT_THEME[theme]}.`}
              title={`Theme: ${theme} (click for ${NEXT_THEME[theme]})`}
            >
              <ThemeIcon />
            </button>
          </div>
        </div>
      </header>
      <main className="page">
        <GameDataProvider>
          <Screen route={route} />
        </GameDataProvider>
      </main>
    </>
  );
}

/** The screen the route names, or what stands in for it. */
function Screen({ route }: { route: Route }) {
  const { state } = useAuth();
  if (route.page === "data") return <DataView />;
  switch (state.status) {
    case "loading":
      return (
        <p className="boot">
          <span className="spinner" /> Checking your session…
        </p>
      );
    case "unknown":
      return (
        <Callout tone="error" title={`Could not check your session (${state.error})`}>
          Reload the page to try again.
        </Callout>
      );
  }
  // The account whose screens are open: the one signed in, or the one whose
  // session ended under them, kept hidden with any unsaved edits until it
  // signs back in. Keyed by account, so nothing one account had open
  // carries over to the next.
  const signedIn = state.status === "signed-in";
  const open = signedIn ? state.account : state.ended;
  return (
    <>
      {open && (
        <div hidden={!signedIn}>
          <Fragment key={open.id}>
            {route.page === "rosters" && <RostersView />}
            {route.page === "base" && <BaseView />}
            {route.page === "plan" && <PlanView />}
            {route.page === "results" && <ResultsView id={route.id} />}
          </Fragment>
        </div>
      )}
      {!signedIn && <SignInView ended={state.ended} />}
    </>
  );
}

/** Who is signed in and a way out, or a way in. */
function AccountActions() {
  const { state, signOut } = useAuth();
  const [busy, setBusy] = useState(false);
  const [failed, setFailed] = useState<string | null>(null);
  if (state.status === "signed-out") {
    return (
      <a className="button secondary sm" href={href("rosters")}>
        Sign in
      </a>
    );
  }
  if (state.status !== "signed-in") return null;
  return (
    <>
      <span className="account-name" title={`Signed in as ${state.account.username}`}>
        {state.account.username}
      </span>
      {failed && (
        <span className="bar-error" role="alert" title={failed}>
          Sign-out failed
        </span>
      )}
      <button
        type="button"
        className="ghost sm"
        disabled={busy}
        onClick={() => {
          setFailed(null);
          setBusy(true);
          signOut().then(
            () => setBusy(false),
            (err: unknown) => {
              setFailed(errorText(err));
              setBusy(false);
            },
          );
        }}
      >
        {busy && <span className="spinner" />}
        Sign out
      </button>
    </>
  );
}
