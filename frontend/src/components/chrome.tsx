// The switches every screen's frame offers, whether that is the header bar
// or the main menu: theme, HUD and signing out.

import { useEffect, useState } from "react";
import { useAuth } from "../auth";
import { errorText } from "../format";
import { recall, remember } from "../storage";
import { IconHud, IconMonitor, IconMoon, IconSun } from "./icons";

export type Theme = "system" | "light" | "dark";
const NEXT_THEME: Record<Theme, Theme> = { system: "light", light: "dark", dark: "system" };

/** The theme picked in this browser; `index.html` applies it before first paint. */
export function useTheme(): [Theme, () => void] {
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

/** The theme and HUD switches, for whichever frame draws them. */
export interface Chrome {
  theme: Theme;
  cycleTheme: () => void;
  hud: boolean;
  toggleHud: () => void;
}

/** Cycles system, light (fog) and dark (night). */
export function ThemeButton({ chrome, className = "ghost icon", size = 18 }: { chrome: Chrome; className?: string; size?: number }) {
  const { theme, cycleTheme } = chrome;
  const Icon = theme === "light" ? IconSun : theme === "dark" ? IconMoon : IconMonitor;
  return (
    <button
      type="button"
      className={className}
      onClick={cycleTheme}
      aria-label={`Theme: ${theme}. Switch to ${NEXT_THEME[theme]}.`}
      title={`Theme: ${theme} (click for ${NEXT_THEME[theme]})`}
    >
      <Icon size={size} />
    </button>
  );
}

/** Shows or hides the PRTS overlay. */
export function HudButton({ chrome, className = "ghost icon", size = 18 }: { chrome: Chrome; className?: string; size?: number }) {
  return (
    <button
      type="button"
      className={className}
      onClick={chrome.toggleHud}
      aria-pressed={chrome.hud}
      aria-label="PRTS overlay"
      title={chrome.hud ? "Hide the PRTS overlay" : "Show the PRTS overlay"}
    >
      <IconHud size={size} />
    </button>
  );
}

/** Signs out, and says so when that fails. */
export function useSignOut(): { busy: boolean; failed: string | null; signOut: () => void } {
  const { signOut } = useAuth();
  const [busy, setBusy] = useState(false);
  const [failed, setFailed] = useState<string | null>(null);
  return {
    busy,
    failed,
    signOut: () => {
      setFailed(null);
      setBusy(true);
      signOut().then(
        () => setBusy(false),
        (err: unknown) => {
          setFailed(errorText(err));
          setBusy(false);
        },
      );
    },
  };
}
