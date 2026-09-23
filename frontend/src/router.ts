// A hash router: `#/page/id`. Pages are the tabs in the header; the id is
// optional (a solve on the results page, for example).

import { useEffect, useState } from "react";

export type Page = "rosters" | "base" | "plan" | "results" | "data";

export const PAGES: { page: Page; label: string }[] = [
  { page: "rosters", label: "Rosters" },
  { page: "base", label: "Base" },
  { page: "plan", label: "Plan" },
  { page: "results", label: "Results" },
  { page: "data", label: "Game data" },
];

export interface Route {
  page: Page;
  id?: string;
}

function parse(hash: string): Route {
  const [first, second] = hash.replace(/^#\/?/, "").split("/").filter(Boolean);
  const page = PAGES.find((p) => p.page === first)?.page ?? "rosters";
  return second ? { page, id: decodeURIComponent(second) } : { page };
}

export function href(page: Page, id?: string): string {
  return id ? `#/${page}/${encodeURIComponent(id)}` : `#/${page}`;
}

export function go(page: Page, id?: string): void {
  window.location.hash = href(page, id);
}

export function useRoute(): Route {
  const [route, setRoute] = useState(() => parse(window.location.hash));
  useEffect(() => {
    const onChange = () => setRoute(parse(window.location.hash));
    window.addEventListener("hashchange", onChange);
    return () => window.removeEventListener("hashchange", onChange);
  }, []);
  return route;
}
