// The PRTS look around every screen: the foggy backdrop behind the page
// and the HUD drawn over it (corner brackets, scanlines, a ruler and a
// status strip). Both are decoration: hidden from assistive technology and
// never in the way of a click. The HUD can be turned off per browser.

import { useEffect, useState } from "react";
import { recall, remember } from "../storage";

/** Whether the HUD is on in this browser, and a switch for it. */
export function useHudSetting(): [boolean, () => void] {
  const [on, setOn] = useState(() => recall("hud") !== "off");
  useEffect(() => remember("hud", on ? null : "off"), [on]);
  return [on, () => setOn((v) => !v)];
}

/** The time now, refreshed every few seconds: for the HUD's readouts. */
export function useClock(): Date {
  const [now, setNow] = useState(() => new Date());
  useEffect(() => {
    const timer = window.setInterval(() => setNow(new Date()), 5_000);
    return () => window.clearInterval(timer);
  }, []);
  return now;
}

const pad = (n: number) => String(n).padStart(2, "0");

/** `2026/10/21 12:30`, the way the game's terminal shows the time. */
export function terminalTime(d: Date): string {
  return `${d.getFullYear()}/${pad(d.getMonth() + 1)}/${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

/**
 * The page behind everything: fog over a blueprint grid. The home screen
 * adds the base's floor plan in faint line; other screens their own name,
 * outlined and huge.
 */
export function Backdrop({ word, plan }: { word?: string; plan?: boolean }) {
  return (
    <div className="backdrop" aria-hidden="true">
      {plan && <FloorPlanLines />}
      {plan && <div className="backdrop-hatch" />}
      {word && <div className="backdrop-word">{word}</div>}
    </div>
  );
}

/** The RIIC's wings and floors, as a faint schematic behind the menu. */
function FloorPlanLines() {
  const wing = [350, 452, 554];
  const floors = [600, 668, 736];
  return (
    <svg className="backdrop-plan" viewBox="0 0 1440 900" preserveAspectRatio="xMidYMid slice">
      <g className="plan-lines">
        {[592, 660, 728, 796].map((y) => (
          <line key={y} x1="250" y1={y} x2="1440" y2={y} />
        ))}
        {floors.map((y) => wing.map((x) => <rect key={`${x}-${y}`} x={x} y={y} width="92" height="52" />))}
        <rect x="684" y="548" width="128" height="104" />
        {floors.map((y) => (
          <rect key={`r-${y}`} x="836" y={y} width="120" height="52" />
        ))}
        <rect x="966" y="600" width="120" height="52" />
        <rect x="966" y="668" width="120" height="52" />
      </g>
      <line className="plan-shaft" x1="664" y1="530" x2="664" y2="820" />
      <g className="plan-labels">
        <text x="300" y="630">B1</text>
        <text x="300" y="698">B2</text>
        <text x="300" y="766">B3</text>
        <text x="692" y="568">CTRL</text>
      </g>
    </svg>
  );
}

/**
 * The overlay. `withBar` starts its frame under the header bar, so the
 * brackets frame the screen's content rather than sit on the bar.
 */
export function Hud({ label, status, withBar }: { label: string; status: string; withBar: boolean }) {
  const now = useClock();
  return (
    <div className={withBar ? "hud with-bar" : "hud"} aria-hidden="true">
      <div className="hud-scan" />
      <div className="hud-frame" />
      <span className="hud-corner tl" />
      <span className="hud-corner tr" />
      <span className="hud-corner bl" />
      <span className="hud-corner br" />
      <div className="hud-ruler" />
      <div className="hud-side">Rhodes Island // Base infrastructure planner</div>
      <div className="hud-status">
        <span className="hud-status-left">
          <span className="hud-dot" />
          PRTS · {label}
        </span>
        <span className="hud-status-mid">{status}</span>
        <span className="hud-status-sys">◆ SYS {terminalTime(now)}</span>
      </div>
    </div>
  );
}
