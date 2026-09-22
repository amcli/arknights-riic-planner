import { useEffect, useState } from "react";
import { api, type GameDataVersion, type OperatorSummary } from "./api";
import { Simulator } from "./Simulator";

type Remote<T> = { state: "loading" } | { state: "error"; message: string } | { state: "ok"; data: T };

function useRemote<T>(load: () => Promise<T>): Remote<T> {
  const [remote, setRemote] = useState<Remote<T>>({ state: "loading" });
  useEffect(() => {
    let cancelled = false;
    load().then(
      (data) => !cancelled && setRemote({ state: "ok", data }),
      (err: unknown) =>
        !cancelled && setRemote({ state: "error", message: err instanceof Error ? err.message : String(err) }),
    );
    return () => {
      cancelled = true;
    };
    // `load` is a stable module-level function in every caller.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  return remote;
}

export function App() {
  const version = useRemote(api.version);
  const operators = useRemote(api.operators);
  const names =
    operators.state === "ok" ? new Map(operators.data.map((o) => [o.id, o.name])) : new Map<string, string>();

  return (
    <main>
      <header>
        <h1>RIIC Planner</h1>
        <p className="muted">Layers 1–4: data ingestion, skill parsing, domain model and simulator. Everything below is served live by <code>ak-api</code>.</p>
      </header>

      <section>
        <h2>Game data</h2>
        {version.state === "loading" && <p>Loading…</p>}
        {version.state === "error" && (
          <p className="error">
            Could not reach the API ({version.message}). Start it with <code>cargo run -p ak-api</code>.
          </p>
        )}
        {version.state === "ok" && <VersionCard v={version.data} />}
      </section>

      <section>
        <h2>Simulator</h2>
        <Simulator names={names} />
      </section>

      <section>
        <h2>Operators</h2>
        {operators.state === "loading" && <p>Loading…</p>}
        {operators.state === "error" && <p className="error">{operators.message}</p>}
        {operators.state === "ok" && <OperatorTable ops={operators.data} />}
      </section>
    </main>
  );
}

function VersionCard({ v }: { v: GameDataVersion }) {
  const commitUrl = `https://github.com/${v.repo}/commit/${v.sha}`;
  const rooms = Object.entries(v.stats.skill_tiers_by_room).sort(([, a], [, b]) => (b ?? 0) - (a ?? 0));
  return (
    <div className="card">
      <dl>
        <dt>Source</dt>
        <dd>
          {v.source} · <a href={commitUrl}>{v.sha.slice(0, 12)}</a> ({v.repo})
        </dd>
        <dt>Fetched</dt>
        <dd>{v.fetched_at ?? "unknown"}</dd>
        <dt>Parser</dt>
        <dd>ak-data {v.parser_version}</dd>
        <dt>Operators</dt>
        <dd>
          {v.stats.operators} loaded
          {v.operators_skipped > 0 && <span className="warn"> · {v.operators_skipped} skipped</span>}
        </dd>
        <dt>Skills</dt>
        <dd>
          {v.stats.skill_tiers} tiers in {v.stats.skill_families} families
        </dd>
        <dt>Parser</dt>
        <dd>
          {v.stats.mechanics_parsed} fully modelled ({v.stats.mechanics_coverage_pct.toFixed(1)}%)
          {v.stats.mechanics_partial > 0 && <span className="muted"> · {v.stats.mechanics_partial} partial</span>}
          {v.stats.mechanics_unparsed > 0 && <span className="warn"> · {v.stats.mechanics_unparsed} rejected</span>}
        </dd>
      </dl>
      <table className="compact">
        <thead>
          <tr>
            <th>Room</th>
            <th>Skill tiers</th>
          </tr>
        </thead>
        <tbody>
          {rooms.map(([room, n]) => (
            <tr key={room}>
              <td>{room}</td>
              <td>{n}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function OperatorTable({ ops }: { ops: OperatorSummary[] }) {
  const [query, setQuery] = useState("");
  const q = query.trim().toLowerCase();
  const shown = ops
    .filter((o) => !q || o.name.toLowerCase().includes(q) || o.id.includes(q))
    .sort((a, b) => b.stars - a.stars || a.name.localeCompare(b.name));
  return (
    <div className="card">
      <input
        type="search"
        placeholder="Filter by name or id"
        value={query}
        onChange={(e) => setQuery(e.target.value)}
        aria-label="Filter operators"
      />
      <p className="muted">
        {shown.length} of {ops.length}
      </p>
      <table>
        <thead>
          <tr>
            <th>Name</th>
            <th>★</th>
            <th>Class</th>
            <th>Subclass</th>
            <th>Nation</th>
            <th>Group</th>
            <th>Team</th>
          </tr>
        </thead>
        <tbody>
          {shown.map((o) => (
            <tr key={o.id}>
              <td title={o.id}>{o.name}</td>
              <td>{o.stars}</td>
              <td>{o.profession}</td>
              <td>{o.sub_profession}</td>
              <td>{o.nation ?? ""}</td>
              <td>{o.group ?? ""}</td>
              <td>{o.team ?? ""}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
