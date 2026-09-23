import { useEffect, useRef, useState } from "react";
import { api, type Candidate, type SolveJob, type SolveProgress, type SolveRequest, type SolveSummary } from "../api";
import { DEFAULT_REQUEST, ResultView } from "./Simulator";

// The simulator's small base, handed to the solver with its assignment as
// the starting point and everyone in the roster as a candidate. Rotation
// over three days, so that morale relief is worth something to the search.
const DEFAULT_SOLVE: SolveRequest = {
  base: DEFAULT_REQUEST.base,
  roster: DEFAULT_REQUEST.roster,
  initial: DEFAULT_REQUEST.assignment,
  locked: [],
  config: { ...DEFAULT_REQUEST.config, horizon_hours: 72 },
  rotation: { kind: "mood_threshold", swap_out: 4, swap_in: 20 },
  objective: { lmd: 1, exp: 1, orundum: 200, drones: 20, contacts: 0, training_hours: 0, gold: 0, exhausted_hour: 0 },
  solver: { seed: 1, strategy: "auto", iterations: 800, restarts: 2, top_k: 5, time_budget_ms: 15000 },
};

type Phase =
  | { state: "idle" }
  | { state: "submitting" }
  | { state: "polling"; id: string; job?: SolveJob }
  | { state: "done"; job: SolveJob }
  | { state: "error"; message: string };

const FINISHED = new Set(["done", "failed", "cancelled"]);

const fmt = (n: number, digits = 0) =>
  n.toLocaleString(undefined, { minimumFractionDigits: digits, maximumFractionDigits: digits });

export function Solver({ names }: { names: Map<string, string> }) {
  const [text, setText] = useState(() => JSON.stringify(DEFAULT_SOLVE, null, 2));
  const [phase, setPhase] = useState<Phase>({ state: "idle" });
  const [recent, setRecent] = useState<SolveSummary[]>([]);
  const timer = useRef<number | null>(null);
  const name = (id: string) => names.get(id) ?? id;

  const refreshRecent = () => {
    api.solves.list().then(setRecent, () => setRecent([]));
  };
  useEffect(() => {
    refreshRecent();
    return () => {
      if (timer.current !== null) window.clearTimeout(timer.current);
    };
  }, []);

  const show = (job: SolveJob) => {
    if (FINISHED.has(job.status)) {
      setPhase({ state: "done", job });
      refreshRecent();
    } else {
      setPhase({ state: "polling", id: job.id, job });
    }
  };

  const poll = (id: string) => {
    if (timer.current !== null) window.clearTimeout(timer.current);
    api.solves.get(id).then(
      (job) => {
        show(job);
        if (!FINISHED.has(job.status)) timer.current = window.setTimeout(() => poll(id), 500);
      },
      (err: unknown) => setPhase({ state: "error", message: err instanceof Error ? err.message : String(err) }),
    );
  };

  const stop = async (id: string) => {
    try {
      show(await api.solves.stop(id));
    } catch (err) {
      setPhase({ state: "error", message: err instanceof Error ? err.message : String(err) });
    }
  };

  const submit = async () => {
    let request: SolveRequest;
    try {
      request = JSON.parse(text) as SolveRequest;
    } catch (err) {
      setPhase({ state: "error", message: `Request is not valid JSON: ${String(err)}` });
      return;
    }
    setPhase({ state: "submitting" });
    try {
      const summary = await api.solves.create(request, `web ${new Date().toLocaleString()}`);
      setPhase({ state: "polling", id: summary.id });
      refreshRecent();
      timer.current = window.setTimeout(() => poll(summary.id), 300);
    } catch (err) {
      setPhase({ state: "error", message: err instanceof Error ? err.message : String(err) });
    }
  };

  const busy = phase.state === "submitting" || phase.state === "polling";

  return (
    <div className="card">
      <p className="muted">
        Searches for better assignments: exhaustive when the space is small, simulated annealing otherwise, with
        the finalists re-scored by the full simulator. Solves run in the background on the server and are kept in
        the store.
      </p>
      <textarea
        className="request"
        value={text}
        onChange={(e) => setText(e.target.value)}
        spellCheck={false}
        aria-label="Solve request JSON"
        rows={12}
      />
      <div className="actions">
        <button type="button" onClick={() => void submit()} disabled={busy}>
          Solve
        </button>
        {phase.state === "polling" && (
          <button
            type="button"
            className="secondary"
            onClick={() => void stop(phase.id)}
            disabled={phase.job?.stop_requested === true}
          >
            {phase.job?.status === "pending" ? "Cancel" : "Stop and keep best"}
          </button>
        )}
        <button type="button" className="secondary" onClick={() => setText(JSON.stringify(DEFAULT_SOLVE, null, 2))}>
          Reset
        </button>
      </div>
      {phase.state === "submitting" && <p>Submitting…</p>}
      {phase.state === "polling" && <Running id={phase.id} job={phase.job} />}
      {phase.state === "error" && <p className="error">{phase.message}</p>}
      {phase.state === "done" && <JobView job={phase.job} name={name} />}
      {recent.length > 0 && (
        <>
          <h3>Recent solves</h3>
          <ul className="events">
            {recent.slice(0, 8).map((s) => (
              <li key={s.id}>
                <button type="button" className="link" onClick={() => poll(s.id)}>
                  {s.name ?? s.id.slice(0, 8)}
                </button>{" "}
                <span className="muted">
                  {s.status} · {new Date(s.created_at).toLocaleString()}
                </span>
              </li>
            ))}
          </ul>
        </>
      )}
    </div>
  );
}

function Running({ id, job }: { id: string; job?: SolveJob }) {
  const status = job?.status ?? "pending";
  const p = job?.progress;
  return (
    <p>
      Job <code>{id.slice(0, 8)}</code> is {status}
      {job?.stop_requested && ", stopping"}
      {status === "pending" && " (waiting for a free slot)"}
      {p ? <> · {describeProgress(p)}</> : "…"}
    </p>
  );
}

function describeProgress(p: SolveProgress): string {
  const pct = p.total > 0 ? ` (${fmt((100 * p.done) / p.total, 1)}%)` : "";
  const seconds = `${fmt(p.elapsed_ms / 1000, 1)} s`;
  if (p.phase === "rescoring") {
    return `re-scoring finalists with the simulator: ${fmt(p.done)} of ${fmt(p.total)} · ${seconds}`;
  }
  const unit = p.strategy === "annealing" ? "steps" : "assignments";
  return (
    `${p.strategy} search: ${fmt(p.done)} of ${fmt(p.total)} ${unit}${pct} · ` +
    `${fmt(p.evaluations)} evaluations · best ${fmt(p.best_score)} vs start ${fmt(p.initial_score)} · ${seconds}`
  );
}

function JobView({ job, name }: { job: SolveJob; name: (id: string) => string }) {
  if (job.status === "failed") return <p className="error">Solve failed: {job.error ?? "unknown error"}</p>;
  if (job.status === "cancelled") return <p className="muted">Solve cancelled before it started.</p>;
  const r = job.result;
  if (!r) return <p className="error">Solve finished without a result.</p>;
  const occupants = (c: Candidate) =>
    Object.entries(c.assignment)
      .map(([room, slots]) => {
        const who = slots.filter((s): s is string => s !== null).map(name);
        return who.length ? `${room}: ${who.join(", ")}` : "";
      })
      .filter(Boolean)
      .join(" · ");
  return (
    <>
      <h3>
        {r.strategy === "exhaustive" ? "Exhaustive" : "Annealing"} search
        <span className="muted">
          {" "}
          · {fmt(r.evaluations)} evaluations, {fmt(r.simulations)} simulations, {fmt(r.elapsed_ms)} ms ·{" "}
          {fmt(r.space.variable_slots)} variable slots, {fmt(r.space.pool)} candidates, about{" "}
          {r.space.estimated_size.toExponential(1)} assignments
        </span>
      </h3>
      <p>
        Starting assignment scored {fmt(r.initial.score)}; best finalist {fmt(r.candidates[0]?.score ?? 0)}.
        {r.stopped && (
          <span className="warn">
            {" "}
            The search was {r.stopped === "time_budget" ? "cut off by its time budget" : "stopped on request"}, so
            these are the best found until then.
          </span>
        )}
      </p>
      <table>
        <thead>
          <tr>
            <th>#</th>
            <th>Score</th>
            <th>LMD</th>
            <th>EXP</th>
            <th>Drones</th>
            <th>Exhausted</th>
            <th>Assignment</th>
          </tr>
        </thead>
        <tbody>
          {r.candidates.map((c, i) => (
            <tr key={i}>
              <td>{i + 1}</td>
              <td>
                {fmt(c.score)}
                {!c.simulated && <span className="muted"> (proxy)</span>}
              </td>
              <td>{fmt(c.breakdown.lmd)}</td>
              <td>{fmt(c.breakdown.exp)}</td>
              <td>{fmt(c.breakdown.drones)}</td>
              <td>{fmt(c.breakdown.exhausted_hours, 1)} h</td>
              <td>{occupants(c)}</td>
            </tr>
          ))}
        </tbody>
      </table>
      {r.best_simulation && (
        <>
          <h3>Best finalist, simulated</h3>
          <ResultView result={r.best_simulation} name={name} />
        </>
      )}
    </>
  );
}
