// The app shell: a header with one tab per screen, and the screen the hash
// names. Game data loads once for all of them.

import { GameDataProvider } from "./data";
import { href, PAGES, useRoute } from "./router";
import { BaseView } from "./views/BaseView";
import { DataView } from "./views/DataView";
import { PlanView } from "./views/PlanView";
import { ResultsView } from "./views/ResultsView";
import { RostersView } from "./views/RostersView";

const INTRO: Record<string, string> = {
  rosters: "Bring in the operators you own. Their promotion decides which base skills are active.",
  base: "Describe your rooms: levels, what each Factory makes, what each Trading Post sells.",
  plan: "Choose a base and a roster, say what counts, and let the solver find assignments.",
  results: "Every solve, with the best assignments it found and what they would produce.",
  data: "Where the numbers come from, and every operator the planner knows.",
};

export function App() {
  const route = useRoute();
  return (
    <>
      <header className="top">
        <div className="top-inner">
          <a className="brand" href={href("rosters")}>
            RIIC Planner
          </a>
          <nav aria-label="Screens">
            {PAGES.map((p) => (
              <a key={p.page} href={href(p.page)} aria-current={route.page === p.page ? "page" : undefined}>
                {p.label}
              </a>
            ))}
          </nav>
        </div>
      </header>
      <main>
        <p className="intro muted">{INTRO[route.page]}</p>
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
