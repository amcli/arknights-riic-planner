// The app shell: a bar with one tab per screen and the theme switch, and
// the screen the hash names. Game data loads once for all of them; each
// screen draws its own header.

import { useEffect, useState, type ComponentType } from "react";
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
import { href, PAGES, useRoute, type Page } from "./router";
import { recall, remember } from "./storage";
import { BaseView } from "./views/BaseView";
import { DataView } from "./views/DataView";
import { PlanView } from "./views/PlanView";
import { ResultsView } from "./views/ResultsView";
import { RostersView } from "./views/RostersView";

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
  const route = useRoute();
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
          {route.page === "rosters" && <RostersView />}
          {route.page === "base" && <BaseView />}
          {route.page === "plan" && <PlanView />}
          {route.page === "results" && <ResultsView id={route.id} />}
          {route.page === "data" && <DataView />}
        </GameDataProvider>
      </main>
    </>
  );
}
