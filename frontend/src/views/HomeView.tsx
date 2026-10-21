// Home: the main menu, laid out like the Arknights terminal. Tiles lead to
// every screen and show what is in each: the base and roster in use, the
// latest solve, the game data's provenance. On the left are the switches,
// notices and the Doctor; in the middle stands the assistant, whose art
// each browser picks for itself.

import { useEffect, useRef, useState, type ChangeEvent } from "react";
import { api, type BaseConfig, type DocumentMeta, type JobStatus, type SolveSummary } from "../api";
import { useAuth } from "../auth";
import { HudButton, ThemeButton, useSignOut, type Chrome } from "../components/chrome";
import { terminalTime, useClock } from "../components/hud";
import {
  IconBase,
  IconCheck,
  IconCode,
  IconData,
  IconDownload,
  IconImage,
  IconLogOut,
  IconPlay,
  IconResults,
  IconRoster,
} from "../components/icons";
import { useGameData } from "../data";
import { ago, errorText, fmt } from "../format";
import { href } from "../router";
import { recall, remember } from "../storage";

interface Docs {
  rosters: DocumentMeta[];
  bases: DocumentMeta[];
  solves: SolveSummary[];
}

const newestFirst = <T extends DocumentMeta>(xs: T[]) => [...xs].sort((a, b) => b.updated_at.localeCompare(a.updated_at));

/** The one picked last on its screen, else the newest. */
const current = (list: DocumentMeta[], picked: string | null) => list.find((d) => d.id === picked) ?? list[0];

/** Trading Posts, Factories and Power Plants: how players name a layout. */
function layoutCode(base: BaseConfig): string {
  const count = (kind: string) => base.rooms.filter((r) => r.kind === kind).length;
  return `${count("TRADING")}-${count("MANUFACTURE")}-${count("POWER")}`;
}

/** What the latest solve's notice says, and its colour. */
const SOLVE_NOTICE: Record<JobStatus, { title: string; tone: string }> = {
  done: { title: "Solve finished", tone: "magenta" },
  running: { title: "Solving", tone: "blue" },
  pending: { title: "Solve queued", tone: "blue" },
  failed: { title: "Solve failed", tone: "red" },
  cancelled: { title: "Solve cancelled", tone: "slate" },
};

export function HomeView({ chrome }: { chrome: Chrome }) {
  const data = useGameData();
  const { state } = useAuth();
  const sign = useSignOut();
  const now = useClock();
  const [docs, setDocs] = useState<Docs | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [base, setBase] = useState<(DocumentMeta & { base: BaseConfig }) | null>(null);
  const [rosterSize, setRosterSize] = useState<number | null>(null);

  const load = () =>
    Promise.all([api.rosters.list(), api.bases.list(), api.solves.list()]).then(
      ([rosters, bases, solves]) => {
        setDocs({
          rosters: newestFirst(rosters),
          bases: newestFirst(bases),
          solves: [...solves].sort((a, b) => b.created_at.localeCompare(a.created_at)),
        });
        setError(null);
      },
      (err: unknown) => setError(errorText(err)),
    );

  useEffect(() => {
    void load();
  }, []);

  const roster = docs ? current(docs.rosters, recall("roster")) : undefined;
  const baseMeta = docs ? current(docs.bases, recall("base")) : undefined;
  const latest = docs?.solves[0];
  const busy = latest?.status === "running" || latest?.status === "pending";

  // While the latest solve runs, look again now and then for it to finish.
  useEffect(() => {
    if (!busy) return;
    const timer = window.setInterval(() => void load(), 5_000);
    return () => window.clearInterval(timer);
  }, [busy]);

  useEffect(() => {
    if (!baseMeta) return setBase(null);
    let cancelled = false;
    api.bases.get(baseMeta.id).then(
      (b) => !cancelled && setBase(b),
      () => !cancelled && setBase(null),
    );
    return () => {
      cancelled = true;
    };
  }, [baseMeta?.id]);

  useEffect(() => {
    if (!roster) return setRosterSize(null);
    let cancelled = false;
    api.rosters.get(roster.id).then(
      (r) => !cancelled && setRosterSize(Object.keys(r.roster).length),
      () => !cancelled && setRosterSize(null),
    );
    return () => {
      cancelled = true;
    };
  }, [roster?.id]);

  const stats = data.version.stats;
  const username = state.status === "signed-in" ? state.account.username : "Doctor";
  const notice = latest ? SOLVE_NOTICE[latest.status] : undefined;

  return (
    <div className="menu">
      <div className="menu-left">
        <div className="menu-buttons">
          <ThemeButton chrome={chrome} className="menu-btn" size={24} />
          <HudButton chrome={chrome} className="menu-btn" size={24} />
          <button
            type="button"
            className="menu-btn"
            onClick={sign.signOut}
            disabled={sign.busy}
            aria-label="Sign out"
            title="Sign out"
          >
            {sign.busy ? <span className="spinner" /> : <IconLogOut size={24} />}
          </button>
        </div>
        {sign.failed && (
          <p className="menu-error" role="alert">
            Sign-out failed: {sign.failed}
          </p>
        )}

        <nav className="notices" aria-label="Notices">
          <a className="notice green" href={href("data")}>
            <span className="notice-icon">
              <IconCheck size={26} />
            </span>
            <span className="notice-text">
              <span className="notice-title">Data snapshot</span>
              <span>{data.version.locale} · pinned</span>
            </span>
          </a>
          <a className="notice olive" href={href("data")}>
            <span className="notice-icon">
              <IconData size={26} />
            </span>
            <span className="notice-text">
              <span className="notice-title">Parser coverage</span>
              <span>
                {fmt(stats.mechanics_parsed)} / {fmt(stats.skill_tiers)} skill tiers
              </span>
            </span>
          </a>
          {latest && notice ? (
            <a className={`notice ${notice.tone}`} href={href("results", latest.id)}>
              <span className="notice-icon">{busy ? <span className="spinner" /> : <IconResults size={26} />}</span>
              <span className="notice-text">
                <span className="notice-title">{notice.title}</span>
                <span className="ellipsis">{latest.name ?? ago(latest.created_at)}</span>
              </span>
            </a>
          ) : (
            <a className="notice slate" href={href("plan")}>
              <span className="notice-icon">
                <IconPlay size={24} />
              </span>
              <span className="notice-text">
                <span className="notice-title">No solves yet</span>
                <span>Plan your first one</span>
              </span>
            </a>
          )}
        </nav>
        {error && (
          <p className="menu-error" role="alert">
            Could not load your documents: {error}
          </p>
        )}

        <div className="doctor">
          <div className="doctor-badge">
            <svg width="168" height="146" viewBox="0 0 168 146" aria-hidden="true">
              <path className="badge-tri" d="M84 4 164 142H4Z" />
              <circle className="badge-ring" cx="84" cy="94" r="40" />
              <path className="badge-arc" d="M84 54A40 40 0 0 1 123 103" />
            </svg>
            <span className="doctor-count">
              <span className="doctor-number">{rosterSize === null ? "—" : fmt(rosterSize)}</span>
              <span className="doctor-unit">OPS</span>
            </span>
          </div>
          <div className="doctor-name">
            <span className="doctor-user">{username}</span>
            <span className="doctor-sub">ROSTER · {roster ? (roster.name ?? "Untitled roster") : "none yet"}</span>
          </div>
        </div>

        <div className="menu-small">
          {latest ? (
            <a className="news" href={href("results", latest.id)}>
              <span className="news-tag">LATEST SOLVE</span>
              <span className="news-title ellipsis">{latest.name ?? "Solve"}</span>
              <span className="news-sub">
                {latest.status} · {ago(latest.created_at)}
              </span>
              <span className="news-dash" aria-hidden="true" />
            </a>
          ) : (
            <a className="news" href={href("plan")}>
              <span className="news-tag">LATEST SOLVE</span>
              <span className="news-title">Nothing solved yet</span>
              <span className="news-sub">Choose a base and a roster, then solve</span>
              <span className="news-dash" aria-hidden="true" />
            </a>
          )}
          <div className="menu-pair">
            <a className="mini-tile" href={href("base")}>
              <IconBase size={20} /> Saved bases <span className="mini-count">{docs ? fmt(docs.bases.length) : "…"}</span>
            </a>
            <a className="mini-tile" href={href("rosters")}>
              <IconRoster size={20} /> Saved rosters <span className="mini-count">{docs ? fmt(docs.rosters.length) : "…"}</span>
            </a>
          </div>
        </div>
      </div>

      <AssistantArt />

      <div className="menu-right">
        <div className="res-bar">
          <span className="res-time">{terminalTime(now)}</span>
          <span className="res">
            <span className="res-icon blue" aria-hidden="true">
              <IconRoster size={14} />
            </span>
            <span className="res-value">{fmt(stats.operators)}</span>
            <span className="res-label">Operators</span>
          </span>
          <span className="res">
            <span className="res-icon diamond" aria-hidden="true" />
            <span className="res-value">{fmt(stats.skill_tiers)}</span>
            <span className="res-label">Skill tiers</span>
          </span>
          <span className="res">
            <span className="res-icon ring" aria-hidden="true" />
            <span className="res-value">{fmt(stats.mechanics_coverage_pct, 1)}%</span>
            <span className="res-label">Modelled</span>
          </span>
        </div>

        <nav className="tiles" aria-label="Screens">
          <a className="tile hero" href={href("plan")}>
            <span className="hero-count">
              <span className="hero-number">{base ? fmt(base.base.rooms.length) : "—"}</span>
              <span className="hero-unit">
                Rooms <span>/ {base ? layoutCode(base.base) : "no base"}</span>
              </span>
            </span>
            <span className="hero-main">
              <span className="tile-title xl">Plan</span>
              <span className="tile-chip light">{base && roster ? "Current" : "Set up"}</span>
              <span className="hero-sub">
                {base ? (base.name ?? "Untitled base") : "No base yet"} · {roster ? (roster.name ?? "Untitled roster") : "no roster yet"}
              </span>
            </span>
            <span className="hero-banner">
              <svg className="banner-plan" viewBox="0 0 250 196" preserveAspectRatio="xMidYMid slice" aria-hidden="true">
                <g>
                  {[16, 62, 108].map((x) => [18, 46, 74].map((y) => <rect key={`${x}-${y}`} x={x} y={y} width="40" height="22" />))}
                  <rect x="160" y="10" width="70" height="52" />
                  <rect x="160" y="74" width="70" height="22" />
                </g>
              </svg>
              <span className="banner-title">
                START SOLVE <IconPlay size={18} />
              </span>
              <span className="banner-sub">Choose effort, then go</span>
            </span>
            <span className="hero-line" aria-hidden="true" />
          </a>

          <a className="tile s3" href={href("rosters")}>
            <IconRoster className="tile-mark" size={118} />
            <span className="tile-title">Rosters</span>
            <span className="tile-sub">
              {docs?.rosters.length
                ? `${fmt(docs.rosters.length)} saved · ${roster?.name ?? "Untitled roster"}`
                : "Import the operators you own"}
            </span>
          </a>

          <a className="tile s3" href={href("base")}>
            <IconBase className="tile-mark" size={118} />
            {docs && docs.bases.length === 0 && <span className="tile-diamond" aria-hidden="true" />}
            <span className="tile-title">Base</span>
            <span className="tile-sub">
              {docs && docs.bases.length === 0
                ? "Describe your rooms first"
                : `${docs ? `${fmt(docs.bases.length)} saved · ` : ""}Floor plan · levels · formulas`}
            </span>
          </a>

          <a className="tile s3 blue" href={latest ? href("results", latest.id) : href("results")}>
            <IconResults className="tile-mark top" size={126} />
            {latest?.status === "done" && (
              <span className="tile-chip green corner">
                <IconCheck size={14} /> Finalists ready
              </span>
            )}
            {busy && (
              <span className="tile-chip cyan corner">
                <span className="spinner" /> Solving
              </span>
            )}
            <span className="tile-title lg">Results</span>
            <span className="tile-sub">Best assignment · per-day output · morale</span>
          </a>

          <div className="tile-group s3 blue">
            <div className="tile-group-head">
              <IconDownload size={20} /> Import roster
            </div>
            <div className="tile-group-body">
              <a className="tile" href={href("rosters", "krooster")}>
                Krooster
              </a>
              <a className="tile" href={href("rosters", "ak-planner")}>
                ak-planner
              </a>
            </div>
          </div>

          <a className="tile s4" href={href("data")}>
            <IconData className="tile-mark" size={118} />
            <span className="tile-chip yellow corner-right">{data.version.locale} · pinned</span>
            <span className="tile-title">Game Data</span>
            <span className="tile-sub">
              {fmt(stats.operators)} operators · {fmt(stats.skill_tiers)} skill tiers, and where the numbers come from
            </span>
          </a>

          <a className="tile s2 deep" href={href("data")}>
            <IconCode className="tile-mark" size={96} />
            <span className="tile-title sm">Dev tools</span>
            <span className="tile-sub">Simulator · Solver</span>
          </a>
        </nav>
      </div>
    </div>
  );
}

const ART_KEY = "assistant-art";

/** Scales a picture to fit 1000 × 1400 and encodes it small enough to keep. */
async function shrink(file: File): Promise<string> {
  const bitmap = await createImageBitmap(file);
  const scale = Math.min(1, 1400 / bitmap.height, 1000 / bitmap.width);
  const canvas = document.createElement("canvas");
  canvas.width = Math.max(1, Math.round(bitmap.width * scale));
  canvas.height = Math.max(1, Math.round(bitmap.height * scale));
  const context = canvas.getContext("2d");
  if (!context) throw new Error("this browser cannot redraw images");
  context.drawImage(bitmap, 0, 0, canvas.width, canvas.height);
  bitmap.close();
  return canvas.toDataURL("image/webp", 0.86);
}

/**
 * The assistant, in the middle of the menu, under a targeting reticle. The
 * art is the viewer's own pick, kept in this browser only.
 */
function AssistantArt() {
  const [art, setArt] = useState<string | null>(() => recall(ART_KEY));
  const [error, setError] = useState<string | null>(null);
  const input = useRef<HTMLInputElement>(null);

  const pick = async (e: ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    e.target.value = "";
    if (!file) return;
    setError(null);
    try {
      const url = await shrink(file);
      remember(ART_KEY, url);
      if (recall(ART_KEY) !== url) throw new Error("the picture is too large to keep in this browser");
      setArt(url);
    } catch (err) {
      setError(errorText(err));
    }
  };

  return (
    <div className="assistant">
      <div className="reticle" aria-hidden="true">
        <svg className="reticle-outer" viewBox="0 0 560 560">
          <circle cx="280" cy="280" r="270" />
          <path d="M280 10a270 270 0 0 1 190 78" />
        </svg>
        <svg className="reticle-inner" viewBox="0 0 560 560">
          <circle cx="280" cy="280" r="226" />
        </svg>
        <svg className="reticle-ticks" viewBox="0 0 560 560">
          <path d="M280 0v40M280 520v40M0 280h40M520 280h40" />
        </svg>
      </div>
      {art ? (
        <img className="assistant-img" src={art} alt="Your assistant operator" />
      ) : (
        <div className="assistant-slot">
          <span className="assistant-tag">ASSISTANT</span>
          <span className="assistant-hint">Your assistant operator stands here.</span>
          <span className="assistant-code">TGT-LOCK · CONTROL CENTER</span>
        </div>
      )}
      <div className="assistant-actions">
        <button type="button" className="secondary sm" onClick={() => input.current?.click()}>
          <IconImage size={15} /> {art ? "Change art" : "Choose art"}
        </button>
        {art && (
          <button
            type="button"
            className="secondary sm"
            onClick={() => {
              remember(ART_KEY, null);
              setArt(null);
            }}
          >
            Remove
          </button>
        )}
        <input ref={input} type="file" accept="image/*" hidden onChange={(e) => void pick(e)} />
      </div>
      {error && (
        <p className="menu-error" role="alert">
          Could not use that picture: {error}
        </p>
      )}
    </div>
  );
}
