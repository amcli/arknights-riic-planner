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
import { useGameData } from "../data";
import { errorText, fmt, professionName, promotion, when } from "../format";
import { EXAMPLE_ROSTER } from "../presets";
import { recall, remember } from "../storage";

const SOURCES: { id: RosterSource; label: string; help: string }[] = [
  {
    id: "krooster",
    label: "Krooster",
    help: "Krooster has no export button. On krooster.com, open the browser console (F12) and run copy(localStorage.getItem(\"v3_roster\")), then paste here. Its older roster format and its database rows are read too.",
  },
  {
    id: "ak-planner",
    label: "ak-planner",
    help: "In GoodEffort's Arknights Planner, use Import/Export at the top right and copy the text. It lists only the operators you are planning for, with no ownership, so each is taken as owned at its current promotion.",
  },
  {
    id: "manual",
    label: "Canonical JSON",
    help: "{ operator id: { promotion: { phase: \"PHASE_2\", level: 90 } } }, the shape requests use. Every id must be in the game data.",
  },
];

/** Reads pasted text, unwrapping a value that was copied as a JSON string. */
function parsePasted(text: string): unknown {
  const value: unknown = JSON.parse(text);
  if (typeof value === "string") return JSON.parse(value);
  return value;
}

export function describeWarning(w: ImportWarning, name: (id: string) => string): string {
  switch (w.kind) {
    case "unknown_operator":
      return `${w.operator}: not in the game data (released later, CN-only, or an alternate form); skipped`;
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

function byPromotion(roster: Roster): string {
  const counts = new Map<string, number>();
  for (const e of Object.values(roster)) {
    const key = e.promotion.phase.replace("PHASE_", "Elite ");
    counts.set(key, (counts.get(key) ?? 0) + 1);
  }
  return [...counts]
    .sort(([a], [b]) => b.localeCompare(a))
    .map(([k, n]) => `${k}: ${n}`)
    .join(" · ");
}

function ReportView({ report, name }: { report: ImportReport; name: (id: string) => string }) {
  return (
    <>
      <p>
        Read {fmt(report.entries)} entries: <strong>{fmt(report.imported)} imported</strong>
        {report.not_owned > 0 && <>, {fmt(report.not_owned)} not owned</>}
        {report.warnings.length > 0 && <>, {fmt(report.warnings.length)} to check</>}.
        <span className="muted"> Format: {report.format.replaceAll("_", " ")}.</span>
      </p>
      {report.warnings.length > 0 && (
        <ul className="warnings">
          {report.warnings.map((w, i) => (
            <li key={i} className="warn">
              {describeWarning(w, name)}
            </li>
          ))}
        </ul>
      )}
    </>
  );
}

function ImportCard({ onSaved }: { onSaved: (id: string) => void }) {
  const { name } = useGameData();
  const [source, setSource] = useState<RosterSource>("krooster");
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

  return (
    <div className="card">
      <h2 className="card-title">Import a roster</h2>
      <fieldset className="segmented" aria-label="Export format">
        {SOURCES.map((s) => (
          <label key={s.id} className={source === s.id ? "on" : undefined}>
            <input
              type="radio"
              name="source"
              value={s.id}
              checked={source === s.id}
              onChange={() => {
                setSource(s.id);
                setPreview(null);
                setError(null);
              }}
            />
            {s.label}
          </label>
        ))}
      </fieldset>
      <p className="muted small">{help}</p>
      <textarea
        className="request"
        rows={8}
        value={text}
        spellCheck={false}
        placeholder="Paste the export here, or choose a file below."
        aria-label="Roster export"
        onChange={(e) => {
          setText(e.target.value);
          setPreview(null);
        }}
      />
      <div className="row">
        <input type="file" accept=".json,.txt,application/json" onChange={(e) => loadFile(e.target.files?.[0])} />
        {source === "manual" && (
          <button
            type="button"
            className="secondary"
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
        <span>Name</span>
        <input type="text" value={label} placeholder="My roster" onChange={(e) => setLabel(e.target.value)} />
      </label>
      <div className="actions">
        <button type="button" className="secondary" disabled={busy || !text.trim()} onClick={() => void run(false)}>
          Preview
        </button>
        <button type="button" disabled={busy || !text.trim()} onClick={() => void run(true)}>
          Save roster
        </button>
      </div>
      {error && <p className="error">{error}</p>}
      {preview && (
        <div className="preview">
          {preview.import ? (
            <ReportView report={preview.import} name={name} />
          ) : (
            <p>{fmt(Object.keys(preview.roster).length)} operators.</p>
          )}
          <p className="muted">{byPromotion(preview.roster)}</p>
        </div>
      )}
    </div>
  );
}

function RosterDetail({ id, active, onUse, onDeleted }: { id: string; active: boolean; onUse: () => void; onDeleted: () => void }) {
  const data = useGameData();
  const [doc, setDoc] = useState<(DocumentMeta & RosterView) | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [query, setQuery] = useState("");

  useEffect(() => {
    setDoc(null);
    setError(null);
    api.rosters.get(id).then(setDoc, (err: unknown) => setError(errorText(err)));
  }, [id]);

  if (error) return <p className="error">{error}</p>;
  if (!doc) return <p className="muted">Loading…</p>;
  const q = query.trim().toLowerCase();
  const rows = Object.entries(doc.roster)
    .map(([opId, entry]) => ({ opId, entry, op: data.operators.get(opId) }))
    .filter(({ opId, op }) => !q || opId.includes(q) || (op?.name.toLowerCase().includes(q) ?? false))
    .sort((a, b) => (b.op?.stars ?? 0) - (a.op?.stars ?? 0) || (a.op?.name ?? a.opId).localeCompare(b.op?.name ?? b.opId));

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
    <div className="card">
      <div className="card-head">
        <h2 className="card-title">{doc.name ?? "Untitled roster"}</h2>
        <div className="actions tight">
          {active ? (
            <span className="badge">Used for planning</span>
          ) : (
            <button type="button" onClick={onUse}>
              Use for planning
            </button>
          )}
          <button type="button" className="secondary danger" onClick={() => void remove()}>
            Delete
          </button>
        </div>
      </div>
      <p className="muted">
        {fmt(Object.keys(doc.roster).length)} operators · from {doc.source} · updated {when(doc.updated_at)}
      </p>
      <p className="muted">{byPromotion(doc.roster)}</p>
      {doc.import && <ReportView report={doc.import} name={data.name} />}
      <input
        type="search"
        placeholder="Filter by name or id"
        value={query}
        onChange={(e) => setQuery(e.target.value)}
        aria-label="Filter operators"
      />
      <table className="compact-rows">
        <thead>
          <tr>
            <th>Operator</th>
            <th>★</th>
            <th>Class</th>
            <th>Promotion</th>
          </tr>
        </thead>
        <tbody>
          {rows.map(({ opId, entry, op }) => (
            <tr key={opId}>
              <td title={opId}>{op?.name ?? opId}</td>
              <td>{op?.stars ?? ""}</td>
              <td>{op ? professionName(op.profession) : ""}</td>
              <td className="num">{promotion(entry)}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
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
    <div className="split">
      <div className="stack">
        <ImportCard
          onSaved={(id) => {
            void refresh();
            setSelected(id);
            use(id);
          }}
        />
        <div className="card">
          <h2 className="card-title">Stored rosters</h2>
          {error && <p className="error">{error}</p>}
          {list && list.length === 0 && <p className="muted">None yet. Import one above.</p>}
          <ul className="doc-list">
            {list?.map((r) => (
              <li key={r.id}>
                <button
                  type="button"
                  className={r.id === shown ? "doc on" : "doc"}
                  onClick={() => setSelected(r.id)}
                  aria-current={r.id === shown}
                >
                  <span>{r.name ?? "Untitled roster"}</span>
                  <span className="muted small">
                    {r.id === active ? "in use · " : ""}
                    {when(r.updated_at)}
                  </span>
                </button>
              </li>
            ))}
          </ul>
        </div>
      </div>
      <div>
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
          <div className="card muted">Import a roster to see it here.</div>
        )}
      </div>
    </div>
  );
}

