// Rosters: import another tool's export (or paste the canonical shape),
// preview what it reads as, save it, and browse what is stored.

import { useEffect, useState } from "react";
import {
  api,
  type DocumentMeta,
  type ImportReport,
  type ImportWarning,
  type Roster,
  type RosterSource,
  type RosterView,
} from "../api";
import {
  IconAlert,
  IconCheck,
  IconDownload,
  IconFile,
  IconRoster,
  IconSave,
  IconSearch,
  IconTrash,
  IconUpload,
} from "../components/icons";
import { Callout, Empty, PageHeader, Segmented } from "../components/ui";
import { useGameData } from "../data";
import { ago, errorText, fmt, professionName, promotion, when } from "../format";
import { EXAMPLE_ROSTER } from "../presets";
import { recall, remember } from "../storage";

const SOURCES: { id: RosterSource; label: string; help: string }[] = [
  {
    id: "krooster",
    label: "Krooster",
    help: "Enter your Krooster username to fetch your roster from your public Krooster profile. Your username ends your profile link in Krooster's Settings (krooster.com/u/…). You can also paste the page at www.krooster.com/api/u/<username> below, or an older Krooster export.",
  },
  {
    id: "ak-planner",
    label: "ak-planner",
    help: "In GoodEffort's Arknights Planner, use Import/Export at the top right and copy the text. It lists only the operators you are planning for, with no ownership, so each is taken as owned at its current promotion.",
  },
  {
    id: "manual",
    label: "JSON",
    help: "{ operator id: { promotion: { phase: \"PHASE_2\", level: 90 } } }, the shape requests use. Every id must be in the game data.",
  },
];

type Phase = Roster[string]["promotion"]["phase"];
const PHASES: Phase[] = ["PHASE_2", "PHASE_1", "PHASE_0"];
const eliteName = (p: Phase) => p.replace("PHASE_", "Elite ");

/** Reads pasted text, unwrapping a value that was copied as a JSON string. */
function parsePasted(text: string): unknown {
  const value: unknown = JSON.parse(text);
  if (typeof value === "string") return JSON.parse(value);
  return value;
}

export function describeWarning(w: ImportWarning, name: (id: string) => string): string {
  switch (w.kind) {
    case "unknown_operator":
      return `${w.operator}: not in the game data (released since it was last updated, CN-only, or an alternate form like Amiya's); skipped`;
    case "promotion_clamped":
      return `${name(w.operator)}: Elite ${w.found} is beyond its rarity; taken as ${w.used.replace("PHASE_", "Elite ")} level ${w.level}`;
    case "level_clamped":
      return `${name(w.operator)}: level ${w.found} is out of range; taken as ${w.used}`;
    case "duplicate":
      return `${name(w.operator)}: listed more than once; the last entry was kept`;
    case "no_saved_plan":
      return `${name(w.operator)}: selected without a saved plan; taken as Elite 0 level 1`;
  }
}

function countByPhase(roster: Roster): Map<Phase, number> {
  const counts = new Map<Phase, number>();
  for (const e of Object.values(roster)) counts.set(e.promotion.phase, (counts.get(e.promotion.phase) ?? 0) + 1);
  return counts;
}

function PromotionStats({ roster }: { roster: Roster }) {
  const counts = countByPhase(roster);
  return (
    <div className="promo-stats">
      {PHASES.filter((p) => counts.has(p)).map((p) => (
        <span key={p}>
          {eliteName(p)} <strong>{fmt(counts.get(p) ?? 0)}</strong>
        </span>
      ))}
    </div>
  );
}

function ReportSummary({ report }: { report: ImportReport }) {
  return (
    <span>
      Read {fmt(report.entries)} entries: <strong>{fmt(report.imported)} imported</strong>
      {report.not_owned > 0 && <>, {fmt(report.not_owned)} not owned</>}
      {report.warnings.length > 0 && <>, {fmt(report.warnings.length)} to check</>}.
      <span className="muted"> Format: {report.format.replaceAll("_", " ")}.</span>
    </span>
  );
}

function ReportWarnings({ report, name, open }: { report: ImportReport; name: (id: string) => string; open?: boolean }) {
  const n = report.warnings.length;
  if (n === 0) return null;
  return (
    <details className="hint" open={open}>
      <summary>
        <IconAlert size={13} className="warn" /> {fmt(n)} {n === 1 ? "entry" : "entries"} to check
      </summary>
      <ul className="notes">
        {report.warnings.map((w, i) => (
          <li key={i}>{describeWarning(w, name)}</li>
        ))}
      </ul>
    </details>
  );
}

function ImportCard({ onSaved }: { onSaved: (id: string) => void }) {
  const { name } = useGameData();
  const [source, setSource] = useState<RosterSource>("krooster");
  const [username, setUsername] = useState(() => recall("krooster-user") ?? "");
  const [text, setText] = useState("");
  const [label, setLabel] = useState("");
  const [preview, setPreview] = useState<RosterView | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const help = SOURCES.find((s) => s.id === source)?.help ?? "";

  const input = (): unknown => {
    try {
      return parsePasted(text);
    } catch (err) {
      throw new Error(`That is not valid JSON: ${errorText(err)}`);
    }
  };

  const run = async (save: boolean) => {
    setError(null);
    setBusy(true);
    try {
      const roster = input();
      if (save) {
        const saved = await api.rosters.create(roster, label.trim() || undefined, source);
        setPreview(null);
        setText("");
        setLabel("");
        onSaved(saved.id);
      } else {
        setPreview(await api.rosters.preview(roster, source));
      }
    } catch (err) {
      setPreview(null);
      setError(errorText(err));
    } finally {
      setBusy(false);
    }
  };

  // Fills the export in like a file would, and previews it.
  const fetchKrooster = async () => {
    const user = username.trim();
    setError(null);
    setBusy(true);
    try {
      const exported = await api.krooster.roster(user);
      remember("krooster-user", user);
      setText(JSON.stringify(exported));
      if (!label) setLabel(user);
      setPreview(await api.rosters.preview(exported, "krooster"));
    } catch (err) {
      setPreview(null);
      setError(errorText(err));
    } finally {
      setBusy(false);
    }
  };

  const loadFile = (file: File | undefined) => {
    if (!file) return;
    file.text().then(
      (t) => {
        setText(t);
        setPreview(null);
        if (!label) setLabel(file.name.replace(/\.[^.]+$/, ""));
      },
      (err: unknown) => setError(errorText(err)),
    );
  };

  const empty = !text.trim();

  return (
    <section className="card">
      <header className="card-head">
        <h2 className="card-title">
          <IconUpload /> Import a roster
        </h2>
      </header>
      <div className="card-body stack-sm">
        <Segmented
          name="source"
          label="Export format"
          full
          value={source}
          options={SOURCES.map((s) => ({ value: s.id, label: s.label }))}
          onChange={(v) => {
            setSource(v);
            setPreview(null);
            setError(null);
          }}
        />
        <details className="hint">
          <summary>Where to find it</summary>
          <p>{help}</p>
        </details>
        {source === "krooster" && (
          <form
            className="row nowrap"
            onSubmit={(e) => {
              e.preventDefault();
              void fetchKrooster();
            }}
          >
            <input
              className="grow"
              type="text"
              value={username}
              placeholder="Krooster username"
              aria-label="Krooster username"
              autoComplete="off"
              spellCheck={false}
              onChange={(e) => setUsername(e.target.value)}
            />
            <button type="submit" className="secondary" disabled={busy || !username.trim()}>
              <IconDownload size={14} /> Fetch
            </button>
          </form>
        )}
        <textarea
          className="mono"
          rows={4}
          value={text}
          spellCheck={false}
          placeholder={source === "krooster" ? "…or paste an export here" : "Paste the export here"}
          aria-label="Roster export"
          onChange={(e) => {
            setText(e.target.value);
            setPreview(null);
          }}
        />
        <div className="row">
          <label className="file-btn sm">
            <IconFile size={14} /> Choose a file
            <input type="file" accept=".json,.txt,application/json" onChange={(e) => loadFile(e.target.files?.[0])} />
          </label>
          {source === "manual" && (
            <button
              type="button"
              className="ghost sm"
              onClick={() => {
                setText(JSON.stringify(EXAMPLE_ROSTER, null, 2));
                setLabel("Example roster");
                setPreview(null);
              }}
            >
              Use the example roster
            </button>
          )}
        </div>
        <label className="field">
          <span className="field-label">Name</span>
          <input type="text" value={label} placeholder="My roster" onChange={(e) => setLabel(e.target.value)} />
        </label>
        <div className="row end">
          <button type="button" className="secondary" disabled={busy || empty} onClick={() => void run(false)}>
            Preview
          </button>
          <button type="button" disabled={busy || empty} onClick={() => void run(true)}>
            <IconSave size={14} /> Save roster
          </button>
        </div>
        {error && <Callout tone="error">{error}</Callout>}
        {preview && (
          <div className="import-preview">
            <span className="ok">
              <IconCheck size={14} />
              {fmt(Object.keys(preview.roster).length)} operators ready to save
            </span>
            {preview.import && <ReportSummary report={preview.import} />}
            <PromotionStats roster={preview.roster} />
            {preview.import && <ReportWarnings report={preview.import} name={name} open />}
          </div>
        )}
      </div>
    </section>
  );
}

function RosterDetail({ id, active, onUse, onDeleted }: { id: string; active: boolean; onUse: () => void; onDeleted: () => void }) {
  const data = useGameData();
  const [doc, setDoc] = useState<(DocumentMeta & RosterView) | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [phase, setPhase] = useState<Phase | "all">("all");

  useEffect(() => {
    setDoc(null);
    setError(null);
    api.rosters.get(id).then(setDoc, (err: unknown) => setError(errorText(err)));
  }, [id]);

  if (error) return <Callout tone="error">{error}</Callout>;
  if (!doc) {
    return (
      <section className="card">
        <p className="boot card-body">
          <span className="spinner" /> Loading…
        </p>
      </section>
    );
  }

  const q = query.trim().toLowerCase();
  const all = Object.entries(doc.roster).map(([opId, entry]) => ({ opId, entry, op: data.operators.get(opId) }));
  const counts = countByPhase(doc.roster);
  const rows = all
    .filter(({ entry }) => phase === "all" || entry.promotion.phase === phase)
    .filter(({ opId, op }) => !q || opId.includes(q) || (op?.name.toLowerCase().includes(q) ?? false))
    .sort((a, b) => (a.op?.name ?? a.opId).localeCompare(b.op?.name ?? b.opId));
  // Grouped by rarity, highest first; ids the data lacks come last.
  const groups = new Map<number, typeof rows>();
  for (const r of rows) {
    const stars = r.op?.stars ?? 0;
    groups.set(stars, [...(groups.get(stars) ?? []), r]);
  }
  const ordered = [...groups].sort(([a], [b]) => b - a);

  const remove = async () => {
    if (!window.confirm(`Delete the roster "${doc.name ?? doc.id}"? This cannot be undone.`)) return;
    try {
      await api.rosters.remove(doc.id);
      onDeleted();
    } catch (err) {
      setError(errorText(err));
    }
  };

  return (
    <section className="card">
      <header className="card-head">
        <div className="min0">
          <h2 className="card-title lg">
            {doc.name ?? "Untitled roster"}
            {active && <span className="badge accent dot">Used for planning</span>}
          </h2>
          <p className="card-sub">
            {fmt(all.length)} operators · from {doc.source} · updated{" "}
            <span title={when(doc.updated_at)}>{ago(doc.updated_at)}</span>
          </p>
        </div>
        <div className="row">
          {!active && (
            <button type="button" className="sm" onClick={onUse}>
              <IconCheck size={14} /> Use for planning
            </button>
          )}
          <button type="button" className="ghost danger sm" onClick={() => void remove()}>
            <IconTrash size={14} /> Delete
          </button>
        </div>
      </header>
      <div className="card-body">
        {doc.import && (
          <div className="import-note">
            <ReportSummary report={doc.import} />
            <ReportWarnings report={doc.import} name={data.name} />
          </div>
        )}
        <div className="toolbar">
          <div className="search">
            <IconSearch size={14} />
            <input
              type="search"
              placeholder="Filter by name or id"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              aria-label="Filter operators"
            />
          </div>
          <div className="chips" role="group" aria-label="Promotion">
            <button type="button" className="chip" aria-pressed={phase === "all"} onClick={() => setPhase("all")}>
              All <span className="n">{fmt(all.length)}</span>
            </button>
            {PHASES.filter((p) => counts.has(p)).map((p) => (
              <button key={p} type="button" className="chip" aria-pressed={phase === p} onClick={() => setPhase(p)}>
                {eliteName(p)} <span className="n">{fmt(counts.get(p) ?? 0)}</span>
              </button>
            ))}
          </div>
          {(q || phase !== "all") && <span className="muted small push">{fmt(rows.length)} shown</span>}
        </div>
        {rows.length === 0 ? (
          <Empty title="No operators match">Try another name, or show every promotion.</Empty>
        ) : (
          <div className="op-groups">
            {ordered.map(([stars, list]) => (
              <section key={stars}>
                <h3 className="op-group-head">
                  {stars > 0 ? `${stars}★` : "Not in the game data"} <span className="count">{list.length}</span>
                </h3>
                <ul className="op-grid">
                  {list.map(({ opId, entry, op }) => (
                    <li
                      key={opId}
                      className="op"
                      title={`${op?.name ?? opId} (${opId})${op ? ` · ${professionName(op.profession)}` : ""} · ${promotion(entry)}`}
                    >
                      <span className="op-name">{op?.name ?? opId}</span>
                      {op && <span className="op-class">{professionName(op.profession)}</span>}
                      <span className={`promo e${entry.promotion.phase.slice(-1)}`}>{promotion(entry)}</span>
                    </li>
                  ))}
                </ul>
              </section>
            ))}
          </div>
        )}
      </div>
    </section>
  );
}

export function RostersView() {
  const [list, setList] = useState<DocumentMeta[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [selected, setSelected] = useState<string | null>(() => recall("roster"));
  const [active, setActive] = useState<string | null>(() => recall("roster"));

  const refresh = () => api.rosters.list().then(setList, (err: unknown) => setError(errorText(err)));
  useEffect(() => {
    void refresh();
  }, []);

  const use = (id: string | null) => {
    remember("roster", id);
    setActive(id);
  };
  const shown = list?.some((r) => r.id === selected) ? selected : (list?.[0]?.id ?? null);

  return (
    <>
      <PageHeader
        title="Rosters"
        description="The operators you own. Their promotion decides which base skills are active."
      />
      <div className="with-sidebar">
        <div className="stack">
          <section className="card">
            <header className="card-head">
              <h2 className="card-title">
                Saved rosters {list && list.length > 0 && <span className="count">{list.length}</span>}
              </h2>
            </header>
            {error && (
              <div className="card-body">
                <Callout tone="error">{error}</Callout>
              </div>
            )}
            {list && list.length === 0 && <p className="doc-empty">None yet. Import one below.</p>}
            {list && list.length > 0 && (
              <ul className="doc-list">
                {list.map((r) => (
                  <li key={r.id}>
                    <button
                      type="button"
                      className={r.id === shown ? "doc on" : "doc"}
                      onClick={() => setSelected(r.id)}
                      aria-current={r.id === shown}
                    >
                      <span className="doc-name">
                        <span>{r.name ?? "Untitled roster"}</span>
                        {r.id === active && <span className="badge accent">In use</span>}
                      </span>
                      <span className="doc-meta" title={when(r.updated_at)}>
                        Updated {ago(r.updated_at)}
                      </span>
                    </button>
                  </li>
                ))}
              </ul>
            )}
          </section>
          <ImportCard
            onSaved={(id) => {
              void refresh();
              setSelected(id);
              use(id);
            }}
          />
        </div>
        <div className="min0">
          {shown ? (
            <RosterDetail
              key={shown}
              id={shown}
              active={shown === active}
              onUse={() => use(shown)}
              onDeleted={() => {
                if (active === shown) use(null);
                setSelected(null);
                void refresh();
              }}
            />
          ) : (
            <section className="card">
              <Empty icon={<IconRoster size={20} />} title="No roster yet">
                Import one from Krooster, ak-planner or JSON, and its operators show up here.
              </Empty>
            </section>
          )}
        </div>
      </div>
    </>
  );
}
