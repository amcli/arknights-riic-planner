// Results: every solve, a running solve's progress, and a finished solve's
// finalists: what each would produce, where everyone goes, how morale
// holds up, and what changes between them.

import { useEffect, useRef, useState } from "react";
import {
  api,
  type AssignmentMap,
  type Candidate,
  type DocumentMeta,
  type SimResult,
  type SolveJob,
  type SolveProgress,
  type SolveSummary,
  type Tagged,
} from "../api";
import { BaseMap, diffRoom } from "../components/BaseMap";
import { MoraleTable } from "../components/MoraleTable";
import { useGameData } from "../data";
import { change, compact, errorText, fmt, itemName, perDay, roomName, when } from "../format";
import { href } from "../router";

const FINISHED = new Set(["done", "failed", "cancelled"]);

function StatusBadge({ status }: { status: string }) {
  return <span className={`badge status-${status}`}>{status}</span>;
}

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
    <div className="card">
      <h2 className="card-title">Solves</h2>
      {error && <p className="error">{error}</p>}
      {list?.length === 0 && (
        <p className="muted">
          No solves yet. <a href={href("plan")}>Plan one</a>.
        </p>
      )}
      {list && list.length > 0 && (
        <table>
          <thead>
            <tr>
              <th>Name</th>
              <th>Status</th>
              <th>Started</th>
              <th />
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
                <td>{when(s.created_at)}</td>
                <td>
                  <button type="button" className="link" onClick={() => void remove(s)}>
                    Delete
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </div>
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

  if (error) return <p className="error">{error}</p>;
  if (!job) return <p className="muted">Loading…</p>;
  const refName = (ref?: string) => (ref ? (names.get(ref) ?? "a deleted document") : "given inline");

  return (
    <div className="stack">
      <div className="card">
        <div className="card-head">
          <div>
            <p className="muted small">
              <a href={href("results")}>All solves</a>
            </p>
            <h2 className="card-title">{job.name ?? "Solve"}</h2>
          </div>
          <StatusBadge status={job.status} />
        </div>
        <p className="muted">
          Base: {refName(job.refs?.base_id)} · roster: {refName(job.refs?.roster_id)} · {fmt(job.request.config.horizon_hours)} h
          horizon · {job.request.rotation.kind === "mood_threshold" ? "rotating tired operators" : "no rotation"} · started{" "}
          {when(job.created_at)}
        </p>
        {!FINISHED.has(job.status) && <Running job={job} onStop={() => void stop()} />}
        {job.status === "failed" && <p className="error">The solve failed: {job.error ?? "unknown error"}</p>}
        {job.status === "cancelled" && (
          <p className="muted">
            Cancelled before it started. <a href={href("plan")}>Plan again</a>.
          </p>
        )}
      </div>
      {job.status === "done" && job.result && <Finished job={job} />}
    </div>
  );
}

function Running({ job, onStop }: { job: SolveJob; onStop: () => void }) {
  const p = job.progress;
  const budget = Number(job.request.solver.time_budget_ms ?? 0);
  return (
    <div className="running">
      {job.status === "pending" && <p>Waiting for a free slot on the server…</p>}
      {p && <ProgressBar p={p} />}
      {p && (
        <p className="muted small">
          {describe(p)}
          {budget > 0 && p.phase === "searching" && <> · stops by {fmt(budget / 1000)} s</>}
        </p>
      )}
      <div className="actions">
        <button type="button" className="secondary" onClick={onStop} disabled={job.stop_requested === true}>
          {job.status === "pending" ? "Cancel" : job.stop_requested ? "Stopping…" : "Stop and keep the best so far"}
        </button>
      </div>
    </div>
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

function Finished({ job }: { job: SolveJob }) {
  const data = useGameData();
  const result = job.result;
  const [selected, setSelected] = useState(0);
  const [baseline, setBaseline] = useState<Baseline>("best");
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
  if (!candidate) return <p className="muted">The solve returned no finalists.</p>;
  const best = result.candidates[0] ?? candidate;
  const startEmpty = Object.values(result.initial.assignment).every((slots) => slots.every((s) => s === null));
  const against: AssignmentMap | undefined =
    baseline === "best" && selected !== 0 ? best.assignment : baseline === "start" && !startEmpty ? result.initial.assignment : undefined;
  const sim = sims.get(selected);
  const base = job.request.base;

  return (
    <>
      <div className="card">
        {result.stopped && (
          <p className="warn">
            The search was {result.stopped === "time_budget" ? "cut off by its time budget" : "stopped on request"}; these
            are the best found until then.
          </p>
        )}
        <Tiles c={candidate} reference={selected === 0 ? result.initial : best} referenceLabel={selected === 0 ? "the start" : "the best"} horizon={horizon} />
        <p className="muted small">
          {result.strategy === "exhaustive" ? "Exhaustive search" : "Simulated annealing"} over {result.space.variable_slots} slots and{" "}
          {result.space.pool} operators (about {result.space.estimated_size.toExponential(1)} arrangements): {fmt(result.evaluations)}{" "}
          quick evaluations, {fmt(result.simulations)} full simulations, {fmt(result.elapsed_ms / 1000, 1)} s. Per-day figures
          are the {fmt(horizon)} h totals scaled to 24 h.
        </p>
      </div>

      <div className="card">
        <h2 className="card-title">Finalists</h2>
        <div className="table-scroll">
          <table className="finalists">
            <thead>
              <tr>
                <th>#</th>
                <th className="num">Score</th>
                <th className="num">vs best</th>
                <th className="num">LMD / day</th>
                <th className="num">EXP / day</th>
                <th className="num">Orundum / day</th>
                <th className="num">Drones / day</th>
                <th className="num">Exhausted h</th>
              </tr>
            </thead>
            <tbody>
              {result.candidates.map((c, i) => (
                <tr key={i} className={i === selected ? "selected" : undefined}>
                  <td>
                    <button type="button" className="link" onClick={() => setSelected(i)} aria-pressed={i === selected}>
                      {i === 0 ? "Best" : `#${i + 1}`}
                    </button>
                  </td>
                  <td className="num">{fmt(c.score)}</td>
                  <td className="num">{i === 0 ? "" : change(c.score, best.score)}</td>
                  <td className="num">{fmt(perDay(c.breakdown.lmd, horizon))}</td>
                  <td className="num">{fmt(perDay(c.breakdown.exp, horizon))}</td>
                  <td className="num">{fmt(perDay(c.breakdown.orundum, horizon))}</td>
                  <td className="num">{fmt(perDay(c.breakdown.drones, horizon))}</td>
                  <td className="num">{fmt(c.breakdown.exhausted_hours, 1)}</td>
                </tr>
              ))}
              <tr className="reference">
                <td>Start</td>
                <td className="num">{fmt(result.initial.score)}</td>
                <td className="num">{change(result.initial.score, best.score)}</td>
                <td className="num">{fmt(perDay(result.initial.breakdown.lmd, horizon))}</td>
                <td className="num">{fmt(perDay(result.initial.breakdown.exp, horizon))}</td>
                <td className="num">{fmt(perDay(result.initial.breakdown.orundum, horizon))}</td>
                <td className="num">{fmt(perDay(result.initial.breakdown.drones, horizon))}</td>
                <td className="num">{fmt(result.initial.breakdown.exhausted_hours, 1)}</td>
              </tr>
            </tbody>
          </table>
        </div>
        <p className="muted small">
          Finalists have distinct scores, so each is a real trade-off. The start is what the solve began from
          {startEmpty ? " (an empty base, since nothing was pinned)" : " (the pinned operators)"}.
        </p>
      </div>

      <div className="card">
        <div className="card-head">
          <h2 className="card-title">{selected === 0 ? "Best assignment" : `Finalist #${selected + 1}`}</h2>
          <label className="inline">
            <span className="small muted">Mark changes against</span>
            <select value={baseline} onChange={(e) => setBaseline(e.target.value as Baseline)}>
              <option value="best">the best</option>
              <option value="start" disabled={startEmpty}>
                the start
              </option>
              <option value="none">nothing</option>
            </select>
          </label>
        </div>
        <BaseMap
          base={base}
          assignment={candidate.assignment}
          baseline={against}
          label={`Where every operator goes in ${selected === 0 ? "the best assignment" : `finalist ${selected + 1}`}`}
        />
        {against && <Changes base={base} now={candidate.assignment} before={against} />}
      </div>

      <div className="card">
        <h2 className="card-title">Rooms, per day</h2>
        {simError && <p className="error">{simError}</p>}
        {!sim && !simError && <p className="muted">Simulating…</p>}
        {sim && <RoomTable sim={sim} horizon={horizon} name={data.name} />}
      </div>

      <div className="card">
        <h2 className="card-title">Morale</h2>
        {sim ? <MoraleTable sim={sim} base={base} assignment={candidate.assignment} /> : <p className="muted">Simulating…</p>}
      </div>

      {sim && sim.warnings.length > 0 && (
        <div className="card">
          <h2 className="card-title">What the model could not honour ({sim.warnings.length})</h2>
          <ul className="warnings">
            {sim.warnings.map((w, i) => (
              <li key={i}>{describeWarning(w, data.name)}</li>
            ))}
          </ul>
        </div>
      )}
    </>
  );
}

function Tiles({ c, reference, referenceLabel, horizon }: { c: Candidate; reference: Candidate; referenceLabel: string; horizon: number }) {
  const tiles = [
    { label: "Score", value: c.score, before: reference.score, perDay: false },
    { label: "LMD per day", value: perDay(c.breakdown.lmd, horizon), before: perDay(reference.breakdown.lmd, horizon), perDay: true },
    { label: "EXP per day", value: perDay(c.breakdown.exp, horizon), before: perDay(reference.breakdown.exp, horizon), perDay: true },
    { label: "Orundum per day", value: perDay(c.breakdown.orundum, horizon), before: perDay(reference.breakdown.orundum, horizon), perDay: true },
    { label: "Drones per day", value: perDay(c.breakdown.drones, horizon), before: perDay(reference.breakdown.drones, horizon), perDay: true },
  ].filter((t) => t.label === "Score" || t.value > 0 || t.before > 0);
  return (
    <div className="tiles">
      {tiles.map((t) => {
        const up = t.value > t.before + 1e-9;
        const down = t.value < t.before - 1e-9;
        return (
          <div key={t.label} className="tile">
            <span className="tile-label">{t.label}</span>
            <span className="tile-value">{compact(t.value)}</span>
            <span className={`tile-delta ${up ? "up" : down ? "down" : ""}`}>
              {up ? "▲" : down ? "▼" : "="} {change(t.value, t.before)} vs {referenceLabel}
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
  if (rows.length === 0) return <p className="muted">No changes.</p>;
  return (
    <ul className="changes">
      {rows.map(({ room, added, removed }) => (
        <li key={room.id}>
          <strong>
            {roomName(room.kind)} {room.id}
          </strong>
          {added.length > 0 && <span className="added"> + {added.map(name).join(", ")}</span>}
          {removed.length > 0 && <span className="removed"> − {removed.map(name).join(", ")}</span>}
        </li>
      ))}
    </ul>
  );
}

function RoomTable({ sim, horizon, name }: { sim: SimResult; horizon: number; name: (id: string) => string }) {
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
    <div className="table-scroll">
      <table>
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
              <td>
                {roomName(r.kind)} <code className="muted">{r.id}</code>
              </td>
              <td className="num">{r.average_stat_pct === null ? "" : `${fmt(r.average_stat_pct)}%`}</td>
              <td>
                {output(r) || <span className="muted">nothing</span>}
                {r.hours_blocked > 0 && <span className="warn small"> · full for {fmt(r.hours_blocked, 1)} h</span>}
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
