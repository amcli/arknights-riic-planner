// A hash router: `#/page/id`. The home screen is the main menu at `#/`;
// the other pages are the tabs in the header. The id is optional (a solve
// on the results page, for example).

import { useEffect, useState } from "react";

export type Page = "home" | "rosters" | "base" | "plan" | "results" | "data";

/** The pages that get a tab in the header: every page but home. */
export const PAGES: { page: Exclude<Page, "home">; label: string }[] = [
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
  const page = PAGES.find((p) => p.page === first)?.page ?? "home";
  return second && page !== "home" ? { page, id: decodeURIComponent(second) } : { page };
}

export function href(page: Page, id?: string): string {
  if (page === "home") return "#/";
  return id ? `#/${page}/${encodeURIComponent(id)}` : `#/${page}`;
}

/** One level up: a page's own list from one of its items, else home. */
export function parentOf(route: Route): string {
  return route.id ? href(route.page) : href("home");
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
