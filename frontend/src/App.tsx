// The app shell, in the style of the Arknights terminal. Home (`#/`) is the
// main menu, which brings its own buttons; every other screen gets a bar
// with a way back, a way home, one tab per screen, the account and the
// switches for theme and HUD. A foggy backdrop sits behind it all and the
// PRTS HUD over it. Game data loads once for all screens and is open to
// everyone; the screens that keep things per account (home among them) ask
// for a sign-in instead while nobody is signed in. Each screen draws its own
// header.

import { Fragment, type ComponentType } from "react";
import { AuthProvider, useAuth } from "./auth";
import { HudButton, ThemeButton, useSignOut, useTheme, type Chrome } from "./components/chrome";
import { Backdrop, Hud, useHudSetting } from "./components/hud";
import {
  IconBase,
  IconChevronLeft,
  IconData,
  IconHome,
  IconLogOut,
  IconPlan,
  IconResults,
  IconRoster,
  type IconProps,
} from "./components/icons";
import { Callout } from "./components/ui";
import { GameDataProvider } from "./data";
import { href, PAGES, parentOf, useRoute, type Page, type Route } from "./router";
import { BaseView } from "./views/BaseView";
import { DataView } from "./views/DataView";
import { HomeView } from "./views/HomeView";
import { PlanView } from "./views/PlanView";
import { ResultsView } from "./views/ResultsView";
import { RostersView } from "./views/RostersView";
import { SignInView } from "./views/SignInView";

const ICONS: Record<Exclude<Page, "home">, ComponentType<IconProps>> = {
  rosters: IconRoster,
  base: IconBase,
  plan: IconPlan,
  results: IconResults,
  data: IconData,
};

/** Per screen: the word outlined behind it, and its name on the HUD. */
const SCREENS: Record<Page, { word?: string; label: string }> = {
  home: { label: "RIIC terminal" },
  rosters: { word: "Rosters", label: "Operator rosters" },
  base: { word: "Base", label: "RIIC layout" },
  plan: { word: "Plan", label: "Operation prep" },
  results: { word: "Results", label: "Solve readout" },
  data: { word: "Data", label: "Archives" },
};

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
  const [hud, toggleHud] = useHudSetting();
  const chrome: Chrome = { theme, cycleTheme, hud, toggleHud };
  // The main menu replaces the bar; until someone signs in, home is the
  // sign-in screen and keeps the bar, so game data stays one click away.
  const menu = route.page === "home" && state.status === "signed-in";
  const screen = SCREENS[route.page];
  const status =
    state.status === "signed-in"
      ? `Signed in as ${state.account.username}`
      : state.status === "loading"
        ? "Checking session"
        : "Awaiting identity verification";
  return (
    <>
      <Backdrop word={menu ? undefined : screen.word} plan={menu} />
      {!menu && (
        <header className="app-bar">
          <div className="app-bar-inner">
            <div className="bar-home">
              <a className="bar-btn" href={parentOf(route)} aria-label="Back" title="Back">
                <IconChevronLeft size={24} />
              </a>
              <a className="bar-btn notch" href={href("home")} aria-label="Main menu" title="Main menu">
                <IconHome size={20} />
              </a>
            </div>
            <a className="brand" href={href("home")}>
              RIIC <span>Planner</span>
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
              <HudButton chrome={chrome} />
              <ThemeButton chrome={chrome} />
            </div>
          </div>
        </header>
      )}
      <main className={menu ? "page home" : "page"}>
        <GameDataProvider>
          <Screen route={route} chrome={chrome} />
        </GameDataProvider>
      </main>
      {hud && <Hud label={screen.label} status={status} withBar={!menu} />}
    </>
  );
}

/** The screen the route names, or what stands in for it. */
function Screen({ route, chrome }: { route: Route; chrome: Chrome }) {
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
            {route.page === "home" && <HomeView chrome={chrome} />}
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
  const { state } = useAuth();
  const { busy, failed, signOut } = useSignOut();
  if (state.status === "signed-out") {
    return (
      <a className="button secondary sm" href={href("home")}>
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
      <button type="button" className="ghost icon" disabled={busy} onClick={signOut} aria-label="Sign out" title="Sign out">
        {busy ? <span className="spinner" /> : <IconLogOut size={18} />}
      </button>
    </>
  );
}
