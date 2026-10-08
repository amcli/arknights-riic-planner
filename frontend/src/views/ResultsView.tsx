// Results: every solve, a running solve's progress, and a finished solve's
// finalists: what each would produce, where everyone goes, how morale
// holds up, and what changes between them.

import { useEffect, useRef, useState } from "react";
import {
  api,
  type AssignmentMap,
  type Breakdown,
  type Candidate,
  type DocumentMeta,
  type SimResult,
  type SolveJob,
  type SolveProgress,
  type SolveSummary,
  type Tagged,
} from "../api";
import { BaseMap, diffRoom, toneOf } from "../components/BaseMap";
import { IconAlert, IconChevronLeft, IconPlus, IconResults, IconStop, IconTrash } from "../components/icons";
import { MoraleTable, type MoraleView } from "../components/MoraleTable";
import { Callout, Empty, PageHeader, Segmented, StatusBadge } from "../components/ui";
import { useGameData } from "../data";
import { ago, change, compact, errorText, fmt, itemName, perDay, roomName, short, when } from "../format";
import { href } from "../router";

const FINISHED = new Set(["done", "failed", "cancelled"]);

export function ResultsView({ id }: { id?: string }) {
  return id ? <SolveDetail key={id} id={id} /> : <SolveList />;
}

function SolveList() {
  const [list, setList] = useState<SolveSummary[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const refresh = () => api.solves.list().then(setList, (err: unknown) => setError(errorText(err)));
  useEffect(() => {
    void refresh();
  }, []);

  const remove = async (s: SolveSummary) => {
    if (!window.confirm(`Delete "${s.name ?? s.id}"? A running solve is stopped first.`)) return;
    try {
      await api.solves.remove(s.id);
      void refresh();
    } catch (err) {
      setError(errorText(err));
    }
  };

  return (
    <>
      <PageHeader
        title="Results"
        description="Every solve, with the best assignments it found and what they would produce."
        actions={
          <a className="button" href={href("plan")}>
            <IconPlus size={14} /> New plan
          </a>
        }
      />
      <div className="stack">
        {error && <Callout tone="error">{error}</Callout>}
        <section className="card">
          {list === null && !error && (
            <p className="boot card-body">
              <span className="spinner" /> Loading…
            </p>
          )}
          {list?.length === 0 && (
            <Empty icon={<IconResults size={20} />} title="No solves yet">
              <a href={href("plan")}>Plan one</a>, and its results land here.
            </Empty>
          )}
          {list && list.length > 0 && (
            <div className="table-wrap">
              <table className="solve-table">
                <thead>
                  <tr>
                    <th>Name</th>
                    <th>Status</th>
                    <th>Started</th>
                    <th>
                      <span className="sr-only">Actions</span>
                    </th>
                  </tr>
                </thead>
                <tbody>
                  {list.map((s) => (
                    <tr key={s.id}>
                      <td>
                        <a href={href("results", s.id)}>{s.name ?? s.id.slice(0, 8)}</a>
                      </td>
                      <td>
                        <StatusBadge status={s.status} />
                      </td>
                      <td className="muted nowrap" title={when(s.created_at)}>
                        {ago(s.created_at)}
                      </td>
                      <td className="num">
                        <button type="button" className="ghost danger sm" onClick={() => void remove(s)}>
                          <IconTrash size={14} /> Delete
                        </button>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </section>
      </div>
    </>
  );
}

function SolveDetail({ id }: { id: string }) {
  const [job, setJob] = useState<SolveJob | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [names, setNames] = useState<Map<string, string>>(new Map());
  const timer = useRef<number | null>(null);

  useEffect(() => {
    let alive = true;
    const poll = () => {
      api.solves.get(id).then(
        (j) => {
          if (!alive) return;
          setJob(j);
          if (!FINISHED.has(j.status)) timer.current = window.setTimeout(poll, 600);
        },
        (err: unknown) => alive && setError(errorText(err)),
      );
    };
    poll();
    const label = (docs: DocumentMeta[]) => docs.map((d) => [d.id, d.name ?? d.id.slice(0, 8)] as const);
    Promise.all([api.bases.list(), api.rosters.list()]).then(
      ([b, r]) => alive && setNames(new Map([...label(b), ...label(r)])),
      () => undefined,
    );
    return () => {
      alive = false;
      if (timer.current !== null) window.clearTimeout(timer.current);
    };
  }, [id]);

  const stop = async () => {
    try {
      setJob(await api.solves.stop(id));
    } catch (err) {
      setError(errorText(err));
    }
  };

  const crumbs = (
    <a href={href("results")}>
      <IconChevronLeft size={14} /> All solves
    </a>
  );
  if (error) {
    return (
      <>
        <PageHeader crumbs={crumbs} title="Solve" />
        <Callout tone="error">{error}</Callout>
      </>
    );
  }
  if (!job) {
    return (
      <p className="boot">
        <span className="spinner" /> Loading…
      </p>
    );
  }
  const refName = (ref?: string) => (ref ? (names.get(ref) ?? "a deleted document") : "given inline");
  const rotation = job.request.rotation;

  return (
    <>
      <PageHeader
        crumbs={crumbs}
        title={
          <>
            {job.name ?? "Solve"} <StatusBadge status={job.status} />
          </>
        }
        meta={
          <ul className="meta-list">
            <li>
              Base <strong>{refName(job.refs?.base_id)}</strong>
            </li>
            <li>
              Roster <strong>{refName(job.refs?.roster_id)}</strong>
            </li>
            <li>{fmt(job.request.config.horizon_hours)} h horizon</li>
            <li>
              {rotation.kind === "mood_threshold"
                ? `rotating tired operators (rest at ${fmt(rotation.swap_out)}, back at ${fmt(rotation.swap_in)})`
                : "no rotation"}
            </li>
            <li title={when(job.created_at)}>started {short(job.created_at)}</li>
          </ul>
        }
      />
      <div className="stack">
        {!FINISHED.has(job.status) && <Running job={job} onStop={() => void stop()} />}
        {job.status === "failed" && (
          <Callout tone="error" title="The solve failed">
            {job.error ?? "unknown error"}
          </Callout>
        )}
        {job.status === "cancelled" && (
          <Callout title="Cancelled before it started">
            <a href={href("plan")}>Plan again</a>.
          </Callout>
        )}
        {job.status === "done" && job.result && <Finished job={job} />}
      </div>
    </>
  );
}

function Running({ job, onStop }: { job: SolveJob; onStop: () => void }) {
  const p = job.progress;
  const budget = Number(job.request.solver.time_budget_ms ?? 0);
  const title =
    job.status === "pending"
      ? "Waiting for a free slot on the server…"
      : p?.phase === "rescoring"
        ? "Re-scoring the finalists"
        : "Searching";
  return (
    <section className="card">
      <div className="card-body">
        <div className="run-head">
          <span className="spinner" />
          <strong>{title}</strong>
          {p && p.total > 0 && <span className="muted small">{fmt(Math.min(100, (100 * p.done) / p.total))}%</span>}
          <button type="button" className="secondary sm push" onClick={onStop} disabled={job.stop_requested === true}>
            <IconStop size={13} />
            {job.status === "pending" ? "Cancel" : job.stop_requested ? "Stopping…" : "Stop and keep the best so far"}
          </button>
        </div>
        {/* Keyed by phase so re-scoring starts a fresh bar instead of sliding back. */}
        {p && <ProgressBar key={p.phase} p={p} />}
        {p && (
          <p className="muted small">
            {describe(p)}
            {budget > 0 && p.phase === "searching" && <> · stops by {fmt(budget / 1000)} s</>}
          </p>
        )}
      </div>
    </section>
  );
}

function describe(p: SolveProgress): string {
  if (p.phase === "rescoring") return `Re-scoring the finalists with the full simulator: ${fmt(p.done)} of ${fmt(p.total)}`;
  const unit = p.strategy === "annealing" ? "steps" : "assignments";
  return `${p.strategy === "annealing" ? "Annealing" : "Exhaustive search"}: ${fmt(p.done)} of ${fmt(p.total)} ${unit} · ${fmt(p.evaluations)} evaluations · best ${fmt(p.best_score)} (start ${fmt(p.initial_score)}) · ${fmt(p.elapsed_ms / 1000, 1)} s`;
}

function ProgressBar({ p }: { p: SolveProgress }) {
  const fraction = p.total > 0 ? Math.min(1, p.done / p.total) : 0;
  const label = p.phase === "rescoring" ? "Re-scoring finalists" : "Searching";
  return (
    <div className="progress" role="progressbar" aria-label={label} aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(fraction * 100)}>
      <div className="progress-fill" style={{ width: `${fraction * 100}%` }} />
    </div>
  );
}

// ---- a finished solve ----------------------------------------------------------

type Baseline = "none" | "best" | "start";

/** Per-day outputs a finalist's summary line can show, in order. */
const OUTPUTS: { key: keyof Breakdown; unit: string }[] = [
  { key: "lmd", unit: "LMD" },
  { key: "exp", unit: "EXP" },
  { key: "orundum", unit: "Orundum" },
  { key: "drones", unit: "drones" },
];

function Finished({ job }: { job: SolveJob }) {
  const data = useGameData();
  const result = job.result;
  const [selected, setSelected] = useState(0);
  const [baseline, setBaseline] = useState<Baseline>("best");
  const [moraleView, setMoraleView] = useState<MoraleView>("chart");
  const [sims, setSims] = useState<Map<number, SimResult>>(() =>
    result?.best_simulation ? new Map([[0, result.best_simulation]]) : new Map(),
  );
  const [simError, setSimError] = useState<string | null>(null);

  useEffect(() => {
    if (sims.has(selected)) return;
    setSimError(null);
    api.solves.simulation(job.id, selected).then(
      (sim) => setSims((m) => new Map(m).set(selected, sim)),
      (err: unknown) => setSimError(errorText(err)),
    );
  }, [selected, sims, job.id]);

  if (!result) return null;
  const horizon = job.request.config.horizon_hours;
  const candidate = result.candidates[selected] ?? result.candidates[0];
  if (!candidate) return <Callout>The solve returned no finalists.</Callout>;
  const best = result.candidates[0] ?? candidate;
  const startEmpty = Object.values(result.initial.assignment).every((slots) => slots.every((s) => s === null));
  const against: AssignmentMap | undefined =
    baseline === "best" && selected !== 0 ? best.assignment : baseline === "start" && !startEmpty ? result.initial.assignment : undefined;
  const sim = sims.get(selected);
  const base = job.request.base;

  // A finalist's outputs per day, naming only what any finalist makes.
  const everyone = [...result.candidates, result.initial];
  const outputs = OUTPUTS.filter((o) => everyone.some((c) => c.breakdown[o.key] > 0));
  const summary = (c: Candidate) => {
    const parts = outputs.map((o) => `${compact(perDay(c.breakdown[o.key], horizon))} ${o.unit}`);
    if (c.breakdown.exhausted_hours > 0) parts.push(`${fmt(c.breakdown.exhausted_hours, 1)} h exhausted in all`);
    return parts.length > 0 ? parts.join(" · ") : "nothing made";
  };

  return (
    <>
      {result.stopped && (
        <Callout tone="warn">
          The search was {result.stopped === "time_budget" ? "cut off by its time budget" : "stopped on request"}; these are
          the best found until then.
        </Callout>
      )}

      <section className="card">
        <Tiles
          c={candidate}
          reference={selected === 0 ? result.initial : best}
          referenceLabel={selected === 0 ? "the start" : "the best"}
          horizon={horizon}
        />
        <p className="card-foot muted small">
          {result.strategy === "exhaustive" ? "Exhaustive search" : "Simulated annealing"} over {result.space.variable_slots} slots and{" "}
          {result.space.pool} operators (about {result.space.estimated_size.toExponential(1)} arrangements): {fmt(result.evaluations)}{" "}
          quick evaluations, {fmt(result.simulations)} full simulations, {fmt(result.elapsed_ms / 1000, 1)} s. Per-day figures are
          the {fmt(horizon)} h totals scaled to 24 h.
        </p>
      </section>

      <section className="card">
        <header className="card-head">
          <h2 className="card-title">{selected === 0 ? "Best assignment" : `Finalist #${selected + 1}`}</h2>
          <label className="inline">
            <span>Mark changes against</span>
            <select value={baseline} onChange={(e) => setBaseline(e.target.value as Baseline)}>
              <option value="best">the best</option>
              <option value="start" disabled={startEmpty}>
                the start
              </option>
              <option value="none">nothing</option>
            </select>
          </label>
        </header>
        <div className="assignment-body">
          <div className="finalists-pane">
            <div className="section-label">
              Finalists <span className="label-note">outputs per day</span>
            </div>
            <ul className="finalists">
              {result.candidates.map((c, i) => (
                <li key={i}>
                  <button type="button" className="finalist" aria-pressed={i === selected} onClick={() => setSelected(i)}>
                    <span className="finalist-rank">{i === 0 ? "Best" : `#${i + 1}`}</span>
                    <span className="finalist-score">{fmt(c.score)}</span>
                    <span className="finalist-delta">{i === 0 ? "" : change(c.score, best.score)}</span>
                    <span className="finalist-line">{summary(c)}</span>
                  </button>
                </li>
              ))}
              <li>
                <div className="finalist reference">
                  <span className="finalist-rank">Start</span>
                  <span className="finalist-score">{fmt(result.initial.score)}</span>
                  <span className="finalist-delta">{change(result.initial.score, best.score)}</span>
                  <span className="finalist-line">{summary(result.initial)}</span>
                </div>
              </li>
            </ul>
            <p className="tiny muted">
              Finalists have distinct scores, so each is a real trade-off. The start is what the solve began from
              {startEmpty ? " (an empty base, since nothing was pinned)" : " (the pinned operators)"}.
            </p>
          </div>
          <div className="map-pane">
            <BaseMap
              base={base}
              assignment={candidate.assignment}
              baseline={against}
              label={`Where every operator goes in ${selected === 0 ? "the best assignment" : `finalist ${selected + 1}`}`}
            />
            {against && <Changes base={base} now={candidate.assignment} before={against} />}
          </div>
        </div>
      </section>

      {/* What each room makes, with the model's caveats beside it when there are any. */}
      <div className={sim && sim.warnings.length > 0 ? "results-lower" : undefined}>
        <section className="card">
          <header className="card-head">
            <h2 className="card-title">Rooms, per day</h2>
          </header>
          {simError && (
            <div className="card-body">
              <Callout tone="error">{simError}</Callout>
            </div>
          )}
          {!sim && !simError && (
            <p className="boot card-body">
              <span className="spinner" /> Simulating…
            </p>
          )}
          {sim && <RoomTable sim={sim} horizon={horizon} />}
        </section>

        {sim && sim.warnings.length > 0 && (
          <details className="card model-notes" open={sim.warnings.length <= 6}>
            <summary>
              <IconAlert size={15} className="warn" /> What the model could not honour{" "}
              <span className="count">{sim.warnings.length}</span>
            </summary>
            <div className="card-body">
              <ul className="notes">
                {sim.warnings.map((w, i) => (
                  <li key={i}>{describeWarning(w, data.name)}</li>
                ))}
              </ul>
            </div>
          </details>
        )}
      </div>

      <section className="card">
        <header className="card-head">
          <div className="min0">
            <h2 className="card-title">Morale</h2>
            <p className="card-sub">0 to 24 over {fmt(horizon)} h, by the room each operator starts in</p>
          </div>
          <Segmented
            name="morale-view"
            label="Morale view"
            value={moraleView}
            options={[
              { value: "chart", label: "Chart" },
              { value: "table", label: "Table" },
            ]}
            onChange={setMoraleView}
          />
        </header>
        <div className="card-body">
          {sim ? (
            <MoraleTable sim={sim} base={base} assignment={candidate.assignment} view={moraleView} />
          ) : (
            <p className="boot">
              <span className="spinner" /> Simulating…
            </p>
          )}
        </div>
      </section>
    </>
  );
}

function Tiles({ c, reference, referenceLabel, horizon }: { c: Candidate; reference: Candidate; referenceLabel: string; horizon: number }) {
  const day = (b: Breakdown, key: keyof Breakdown) => perDay(b[key], horizon);
  const tiles = [
    { label: "Score", value: c.score, before: reference.score, always: true, upIsGood: true },
    { label: "LMD per day", value: day(c.breakdown, "lmd"), before: day(reference.breakdown, "lmd"), upIsGood: true },
    { label: "EXP per day", value: day(c.breakdown, "exp"), before: day(reference.breakdown, "exp"), upIsGood: true },
    { label: "Orundum per day", value: day(c.breakdown, "orundum"), before: day(reference.breakdown, "orundum"), upIsGood: true },
    { label: "Drones per day", value: day(c.breakdown, "drones"), before: day(reference.breakdown, "drones"), upIsGood: true },
    {
      label: "Hours exhausted",
      value: c.breakdown.exhausted_hours,
      before: reference.breakdown.exhausted_hours,
      upIsGood: false,
    },
  ].filter((t) => t.always || t.value > 0 || t.before > 0);
  return (
    <div className="kpis">
      {tiles.map((t) => {
        const up = t.value > t.before + 1e-9;
        const down = t.value < t.before - 1e-9;
        const good = (up && t.upIsGood) || (down && !t.upIsGood);
        const bad = (down && t.upIsGood) || (up && !t.upIsGood);
        return (
          <div key={t.label} className="kpi">
            <span className="kpi-label">{t.label}</span>
            <span className="kpi-value">{compact(t.value)}</span>
            <span className={`kpi-delta ${good ? "up" : bad ? "down" : ""}`}>
              <span aria-hidden="true">{up ? "▲" : down ? "▼" : "="}</span>
              {change(t.value, t.before)} <span className="vs">vs {referenceLabel}</span>
            </span>
          </div>
        );
      })}
    </div>
  );
}

function Changes({ base, now, before }: { base: SolveJob["request"]["base"]; now: AssignmentMap; before: AssignmentMap }) {
  const { name } = useGameData();
  const rows = base.rooms
    .map((room) => ({ room, ...diffRoom(room.id, now, before) }))
    .filter((r) => r.added.length > 0 || r.removed.length > 0);
  if (rows.length === 0) return <p className="muted small">No changes.</p>;
  return (
    <div>
      <div className="section-label">
        Changes <span className="count">{rows.length}</span>
      </div>
      <ul className="changes">
        {rows.map(({ room, added, removed }) => (
          <li key={room.id}>
            <span className="room">
              {roomName(room.kind)} <span className="id">{room.id}</span>
            </span>
            {added.length > 0 && <span className="added"> + {added.map(name).join(", ")}</span>}
            {removed.length > 0 && <span className="removed"> − {removed.map(name).join(", ")}</span>}
          </li>
        ))}
      </ul>
    </div>
  );
}

function RoomTable({ sim, horizon }: { sim: SimResult; horizon: number }) {
  const { name, facilities } = useGameData();
  const day = (v: number) => perDay(v, horizon);
  const output = (r: SimResult["rooms"][number]) => {
    const parts: string[] = [];
    for (const [item, n] of Object.entries(r.produced)) parts.push(`${fmt(day(n), 2)} ${itemName(item)}`);
    if (r.lmd > 0) parts.push(`${fmt(day(r.lmd))} LMD from ${fmt(day(r.orders_completed), 1)} orders`);
    if (r.orundum > 0) parts.push(`${fmt(day(r.orundum))} Orundum`);
    if (r.drones > 0) parts.push(`${fmt(day(r.drones))} drones`);
    if (r.contacts > 0) parts.push(`${fmt(day(r.contacts), 2)} contacts`);
    if (r.training_progress_hours > 0) parts.push(`${fmt(r.training_progress_hours, 1)} h of training in all`);
    if (r.kind === "MEETING") parts.push("clue output is not simulated");
    return parts.join(" · ");
  };
  // Rooms that make something; the Control Center, Dormitories and the
  // Workshop show up in the morale table instead.
  const rows = sim.rooms.filter((r) => r.average_stat_pct !== null);
  return (
    <div className="table-wrap">
      <table className="dense">
        <thead>
          <tr>
            <th>Room</th>
            <th className="num">Speed</th>
            <th>Output per day</th>
            <th>Stationed at the end</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((r) => (
            <tr key={r.id}>
              <td className="nowrap">
                <span className="room-cell">
                  <span className={`swatch ${toneOf(r.kind, facilities)}`} aria-hidden="true" />
                  {roomName(r.kind)} <span className="id">{r.id}</span>
                </span>
              </td>
              <td className="num">{r.average_stat_pct === null ? "" : `${fmt(r.average_stat_pct)}%`}</td>
              <td className="output-cell">
                {output(r) || <span className="muted">nothing</span>}
                {r.hours_blocked > 0 && <span className="blocked"> · full for {fmt(r.hours_blocked, 1)} h</span>}
              </td>
              <td>{r.operators.map(name).join(", ")}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function describeWarning(w: Tagged, name: (id: string) => string): string {
  const { kind, ...rest } = w;
  const text = (k: string) => (typeof rest[k] === "string" ? (rest[k] as string) : "");
  switch (kind) {
    case "unmodeled_effect":
      return `${text("skill")}: ${text("summary")} (not modelled; applied nothing)`;
    case "unmodeled_predicate":
      return `${text("skill")}: condition "${text("text")}" is not modelled; treated as false`;
    case "unmodeled_counter":
      return `${text("skill")}: counter "${text("text")}" is not modelled; counted zero`;
    case "resource_not_simulated":
      return `${text("skill")}: ${text("resource")} is not simulated yet`;
    case "clue_bias_ignored":
      return `${text("skill")}: clue-type bias has no numeric model`;
    case "operator_not_in_roster":
      return `${name(text("operator"))} is not in the roster; assumed fully raised`;
    case "idle_room":
      return `${roomName(text("room_kind") as never)} ${text("room")} has nothing to do`;
    case "input_bound_formula":
      return `Factory ${text("room")}: a Dualchip formula needs input stock; it made nothing`;
    case "unknown_tag":
      return `${text("skill")}: the group "${text("tag")}" is not defined in the data; counted nobody`;
    default: {
      const fields = Object.entries(rest)
        .map(([k, v]) => `${k}: ${String(v)}`)
        .join(", ");
      return fields ? `${kind.replaceAll("_", " ")} (${fields})` : kind.replaceAll("_", " ");
    }
  }
}
