# arknights-riic-planner

An accuracy-first optimiser for the Arknights RIIC (base). Rust workspace
backend, React frontend. See [`docs/plan.md`](docs/plan.md) for the full
design; this README tracks what actually exists.

## Status

| Layer | Crate | State |
| --- | --- | --- |
| 1. Data ingestion | `ak-data`, `ak-data-sync` | **Done.** Pinned upstream snapshot, schema drift check, strict two-stage transform. |
| 2. Skill DSL | `ak-data::mechanics` | **Done to 92% / 100%.** Description parser → typed `Mechanics` AST. 589 of 640 tiers fully modelled, 51 partial with named gaps, 0 rejected. |
| 3. Domain model | `ak-domain` | **Done.** `BaseConfig` (validated against room limits, layout slots and power), `Assignment` (checked mutators, so it cannot hold an impossible state), `Roster`. |
| 4. Evaluator / mood sim | `ak-eval` | **Done for production and morale.** Instantaneous evaluator plus a deterministic fixed-tick simulator with rotation, collection and a shared depot. Clue output and resource accumulators are not simulated yet. |
| 5. Solver | `ak-solver` | **Done.** Exhaustive search for small spaces, simulated annealing otherwise; a fast steady-state proxy inside the loop, the full simulator on the finalists; top-K with distinct scores; deterministic. |
| 6. Persistence | `ak-store` | **Done, file-backed.** Versioned JSON documents for rosters, bases and solve jobs, migrated on read. The Postgres backend the plan calls for waits for a database; the trait is in place. |
| 7. API | `ak-api` | Game-data endpoints, `evaluate` and `simulate`, CRUD for rosters and bases, and solves as background jobs with polling. |
| 8. Roster import | — | Not started. |
| 9. Frontend | `frontend/` | Vite + React + TS: game-data overview, a simulator panel, a solver panel with job polling, and the operator list. |

**Ingestion coverage (pinned en_US snapshot):** 374 / 374 operators with
base skills, 640 / 640 skill tiers, 0 skipped under strict mode.

**Parser coverage (same snapshot):**

| Outcome | Tiers | Meaning |
| --- | --- | --- |
| Fully modelled | 589 (92.0%) | Every predicate, counter and effect is a typed AST node the evaluator can act on. |
| Partial | 51 (8.0%) | Parsed, but at least one part is an explicit `Unmodeled` node (e.g. "chance of higher-yield gold orders is increased"). The quantified parts are still usable. |
| Rejected | 0 | The parser could not read the description at all. |

Every parsed magnitude that upstream also exposes through its display-only
`efficiency` sort hint agrees with that hint (over 200 tiers cross-checked
in `crates/ak-data/tests/mechanics.rs`). The coverage numbers are a test
ratchet: they may only go up.

## How Layer 2 works

Upstream ships **no** structured effect data. Each buff has an id, names,
icons, sort keys, a display-only `efficiency` percentage with `targets`, and
the localised description string. The mechanics live only in that string,
but it is highly templated, so the parser is compositional rather than one
rule per sentence:

1. **Templatize** the rich text: `<@cc.vup>+15%</>` → `{V:+15%}`, keywords
   → `{K:Guard}`, glossary terms → `{T:cc.g.bs}` (term ids are stable across
   locales; their display text is not).
2. **Prefix**: strip "When this Operator is assigned to …", harvesting any
   condition it carries ("to the same Trading Post as Texas") or the Workshop
   material filter.
3. **Notes**: harvest stacking ("only the strongest effect of this type") and
   caps from parentheticals, then drop them.
4. **Split** into clauses at `;`, `. `, `, and`, `, but`, ` and `, plain
   commas — only when the following text starts like a new clause, so
   "Caster and Medic Operators'" and "if X, Y" stay whole. A few sentences
   the splitter would mangle are matched whole by *compound* rules first.
5. **Decompose** each clause: leading/trailing condition, leading/trailing
   counter ("for each Blacksteel operator in this Factory", "per Dormitory
   level"), ramps ("+20% in the first hour, then +1%/h up to +25%"), "up to
   N" caps, and "an additional N" clauses that inherit the previous effect's
   kind.
6. **Match** the core phrase against the effect rule table.

The parser never guesses. An unknown phrase rejects the whole tier; a known
but unquantifiable phrase becomes an `Unmodeled` node, so the evaluator can
warn instead of silently applying nothing. The exotic accumulator systems
(Worldly Plight, Perception Information, Felvine, …) are modelled as named
resources with gain / convert / scale-by effects, so they parse honestly and
a later layer can decide how far to simulate them.

Dev loop for improving coverage:

```bash
cargo run -p ak-cli -- mechanics --partial       # rejected + partial tiers
cargo run -p ak-data --example coverage           # rejects grouped by reason
cargo run -p ak-cli -- skill "control_bd_spd[000]"  # one tier, parsed clauses
cargo run -p ak-data --example inventory          # which AST nodes occur, and how often
```

Known modelling gaps (all reported as partial, none silently dropped):
qualitative clue-type biases, "higher-yield gold order" chance, Workshop
byproduct substitution, morale swap / immunity effects, and counters over
things the model does not track yet (orders below the limit, dorm operators
below a morale threshold).

## How Layers 3 and 4 work

### Describing a base

A simulation request has five parts. Complete examples live in
[`examples/requests/`](examples/requests/).

- **`base`**: rooms with a label, kind, level and settings (Factory
  `formula`, Trading Post `strategy`, Dormitory `ambience`, Training Room
  `training` job). `BaseConfig::validate` rejects anything the game would not
  allow: a level the room kind lacks, more rooms of a kind than its
  `maxCount`, more rooms of a category than the layout has slots (9
  production, 4 dormitory, 4 function, 1 Control Center), more power drawn
  than the Power Plants supply, a formula above the room's level, or a
  setting on the wrong kind of room.
- **`assignment`**: for every room, one entry per slot, each an operator id
  or `null`. The type never holds an operator twice, and the solver changes
  it only through `place`, `remove`, `move_to` and `swap`, which keep that
  true. In a Training Room, slot 0 is the assistant and slot 1 the trainee.
- **`roster`**: each owned operator's promotion, which picks the active tier
  of each skill slot, and optionally their current morale. Stationed
  operators missing from the roster are assumed fully raised, with a warning.
- **`config`**: horizon (default 24 h), tick (default 5 min), starting-morale
  policy, collection policy, starting Pure Gold and Shards, optional Factory
  input stock, and membership tables for glossary groups the data does not
  define.
- **`rotation`**: `none` (the default) or `mood_threshold`.

### Evaluating one instant

`ak_eval::evaluate` turns every stationed operator's parsed skill clauses
into room stats and morale rates at the starting morale, and lists every
contribution and every warning. The solver ranks with it. Semantics that
the AST alone does not settle:

- **Stacking.** "Only the strongest effect of this type" is per effect type,
  not per skill. Eleven Dormitory skill families share the whole-room
  recovery type, and two Control Center families both give +7% to all
  Trading Posts; in each group only the largest applies.
- **Scaling.** "The productivity contributed by all other Operators in that
  Factory becomes 0 (excluding productivity granted based on facility
  count)" zeroes the skill contributions of the other operators in that
  room only. Control Center bonuses, facility-count bonuses and the
  per-operator base bonus remain.
- **Group counters** include the skill owner unless the text says "other".
  Four Team Rainbow operators bring the Control Center's drain to exactly
  zero only if each counts themself. Control Center skills such as "all
  Knight Operators assigned to Factories gain productivity" count each
  Factory's own Knights.
- **Exhaustion.** At zero morale an operator stays in the slot and still
  counts toward headcount relief, but applies no skills and no base bonus.
- **Idle operators.** Training Room trainees, and assistants with no
  training running, neither work nor rest.

### Simulating a horizon

`ak_eval::simulate` repeats: evaluate, produce for one tick, move morale,
collect if due, ask the rotation policy. Nothing is random; Trading Post
orders use expected values.

| Room | Rate at 100% | Limits |
| --- | --- | --- |
| Factory | `count × 3600 / costPoint` units per hour | storage volume under periodic collection (Pure Gold takes 2, Tactical records 5); optional input stock |
| Trading Post | one order per expected order time (203.4 min at level 3) | order limit; Pure Gold or Shards in the depot, shared between posts in proportion to demand |
| Power Plant | 10 drones per hour | none |
| Office | one contact per 12 hours | 3 stored, under periodic collection |
| Training Room | one base hour of progress per hour | stops when the level completes (8, 16 or 24 base hours) |

Working rooms drain 1.0 morale per hour. Factories and Trading Posts drain
0.05 less with two operators and 0.10 less with three, and every active
Control Center operator takes 0.05 per hour off every working operator.
Dormitories restore 1.5 + 0.1 × level + 0.0004 × ambience per hour.

`mood_threshold` rotation sends a working operator to a free Dormitory slot
at `swap_out` morale, fills the slot with the best-rested bench operator at
or above `swap_in`, and brings the original back at `swap_in`.

Every number that is not in the data files is in
[`crates/ak-eval/src/rules.rs`](crates/ak-eval/src/rules.rs) with its source
(PRTS, arknights.wiki.gg, or `item_table.json` at the pinned commit), and
[`crates/ak-eval/tests/scenarios.rs`](crates/ak-eval/tests/scenarios.rs) pins
each one to the number it must produce.

### Not modelled yet

Each of these produces a warning when it matters, rather than a silent zero:

- Reception Room clue output. Its speed also depends on each operator's
  rarity and promotion, total Dormitory ambience and room level.
- Resource accumulators (Worldly Plight, Perception Information, …) and
  qualitative clue biases.
- The chance of higher-yield gold orders.
- Glossary groups the data does not define (Knights, Operation Platforms,
  Soubo Adventurers, …). Supply them in `config.memberships`; otherwise
  counters over them count nobody.
- Workshop crafting. Its skills are reported but not simulated.

Also not modelled, without a warning: room unlock conditions (Control Center
level gating), and the Office operator pausing work when 3 contacts are
stored.

## How Layer 5 works

A solve request ([`examples/requests/solve-243.json`](examples/requests/solve-243.json))
is a base, a roster, an optional starting assignment, locked slots, the
simulation config and rotation, an `objective`, and `solver` settings.

**Search space.** Every slot the request leaves free is a variable, filled
from the roster (or a narrower `pool`). Training Room trainee slots are
always fixed. Moves are: swap two slots, replace an occupant with a bench
operator, fill an empty slot, clear a slot. Every move goes through
`Assignment`'s checked mutators, so no candidate is ever invalid. Slots
within a room are interchangeable, so swaps within a room are never
proposed and rearrangements within a room count as one assignment.

**Strategies.** `auto` enumerates the space exhaustively when it has at most
`exhaustive_limit` (20 000) distinct assignments, and runs simulated
annealing otherwise: geometric cooling from a temperature picked from the
typical size of one move's effect, `restarts` independent runs from the
starting assignment, an optional `time_budget_ms`. The exhaustive search is
the ground truth the annealer is tested against.

**Two scorers.** Inside the loop, candidates are scored by a steady-state
proxy: the instantaneous evaluator, extrapolated over the horizon with the
simulator's own production rules and a shared depot (Trading Posts can sell
no more gold than the Factories make). Morale is priced as time. Without
rotation an operator's contribution stops when their morale would hit zero.
Under rotation they work until `swap_out` and then alternate work and rest,
so their contribution is scaled by `recovery / (recovery + drain)`, where
`recovery` is the best Dormitory's rate. The proxy takes about 0.2 ms; a
24-hour simulation about 40 ms. The finalists are then re-scored by the full
simulator and ranked by that, with the best one's simulation returned in
full. On the example bases the proxy lands within 0.5% of the simulator.

**Objective.** A weighted sum over the horizon. Defaults are cost-based
where a cost exists and documented in `ak_solver::Objective`: LMD and EXP at
1 each, Orundum at 200 (what two Shards cost in LMD and Factory time),
drones at 20 (three minutes of Factory time each), and contacts, training,
unsold gold and exhausted hours at 0. Set your own.

**Results.** The top `top_k` finalists with distinct scores, each with its
breakdown; the starting assignment is always among them, so a solve can
never report something worse than what it was given. The same request and
seed give the same answer.

One consequence to know about: within a 24-hour horizon at full morale,
morale is free, so without rotation the solver will happily dismantle a
Control Center morale crew to put bodies in Factories. Enable rotation or
lengthen the horizon and the crew earns its keep: on the example base with
`mood_threshold` rotation over 72 hours, the solver keeps Team Rainbow in
the Control Center and lifts the score from 176 k to 227 k in about two
seconds (6 100 proxy evaluations, 7 simulations).

## How Layer 6 works

`ak-store` keeps rosters, bases and solve jobs as documents: an id, a
name, timestamps, a `schema_version` and a JSON body. Bodies are never
decomposed into columns. A document from an older schema is migrated on
read; one from a newer build is refused rather than guessed at.

The only backend is a directory of JSON files, one per document, each
write atomic (temp file, then rename). It needs no database and no C
toolchain, both of which this machine lacks; a Postgres JSONB backend can
replace it behind the same `Store` trait. The API opens the store at
`<workspace>/store` (`--store-dir` or `AK_STORE_DIR`), which is git-ignored.

Solves are background jobs: `POST /api/v1/solves` validates the request the
way the solver would, stores it as `pending`, answers 202, and runs it on a
blocking thread; the document moves through `running` to `done` with the
result or `failed` with the reason. Poll `GET /api/v1/solves/{id}`.

## Layout

```
Cargo.toml              workspace (edition 2024, resolver 3)
data/
  manifest.toml         pinned upstream sources (repo + commit SHA)
  en_US/                fetched files + .sync.json provenance sidecar
crates/
  ak-domain/            newtyped IDs, closed enums, GameData, Mechanics AST,
                        BaseConfig, Assignment, Roster
  ak-data/              raw serde mirrors → schema check → strict transform
    src/mechanics/      Layer 2: template, prefix, clause, rules, terms
  ak-data-sync/         fetch pinned SHA, verify digests, validate
  ak-eval/              Layer 4: evaluator + mood simulator (pure, no I/O)
    src/rules.rs        every rule not in the data, with its source
    tests/scenarios.rs  golden scenarios, one per rule
  ak-solver/            Layer 5: space, exhaustive, anneal, objective, evaluator
    tests/solver.rs     exhaustive vs annealing, determinism, locks, budget
  ak-store/             Layer 6: versioned JSON documents, file-backed
  ak-api/               axum: gamedata, evaluate, simulate, rosters, bases, solves
  ak-cli/               `ak stats | op | skill | find | skipped | mechanics | simulate | solve`
examples/requests/      simulation and solve requests for a 2-4-3 base
frontend/               Vite + React + TypeScript
```

Dependency direction is strictly `ak-domain ← ak-data ← {sync, cli, api}`,
`ak-domain ← ak-eval ← ak-solver ← {cli, api}` and `ak-store ← ak-api`.
`ak-eval` and `ak-solver` must never grow an async, I/O or framework
dependency.

## Upstream data

Source: [Kengxxiao/ArknightsGameData_YoStar](https://github.com/Kengxxiao/ArknightsGameData_YoStar),
`en_US`, commit `57010cb5b2af` (2025-11-13, client 31.5.80). That repository
was archived on that date, so this is the final Global snapshot from it; the
CN repository is still live and is listed (disabled) in the manifest with an
identical schema. Operators released to Global after November 2025 are absent
until a successor EN source is chosen. Note that the Layer 2 parser is
written against English descriptions; a CN source would need a second rule
table (term ids and values are shared, prose is not).

Files ingested: `building_data.json`, `character_table.json`,
`handbook_team_table.json` (~16 MB, committed).

### Bumping the pin

1. Edit the `sha` in `data/manifest.toml`.
2. `cargo run -p ak-data-sync -- sync` — fetches, writes `.sync.json`, runs
   the schema drift check and a strict transform.
3. `cargo test --workspace` — update the expectations in
   `crates/ak-data/tests/pinned_data.rs` and the coverage ratchet in
   `crates/ak-data/tests/mechanics.rs` deliberately.
4. Commit the manifest, the data, and the test changes together.

The loader refuses to start if the on-disk sidecar's SHA differs from the
manifest, so a half-done bump fails loudly.

## Running

```bash
cargo test --workspace                     # 126 tests, a few seconds after the first build
cargo run -p ak-data-sync -- check         # verify pins, digests, schema, transform
cargo run -p ak-cli -- stats               # counts + parser coverage as JSON
cargo run -p ak-cli -- op char_285_medic2  # one operator, resolved skills
cargo run -p ak-cli -- simulate examples/requests/243-base.json             # 24 h report
cargo run -p ak-cli -- simulate --evaluate examples/requests/243-base.json  # starting instant
cargo run -p ak-cli -- solve examples/requests/solve-243.json               # search, ~2 s
cargo run -p ak-api                        # http://127.0.0.1:8080, store in ./store
```

```bash
cd frontend && npm install && npm run dev  # proxies /api to the backend
```

API endpoints today:

- `GET /healthz`
- `GET /api/v1/gamedata/version` — source, SHA, fetch time, parser version, counts, coverage
- `GET /api/v1/gamedata/operators` — summary list
- `GET /api/v1/gamedata/operators/{id}`
- `GET /api/v1/gamedata/skills/{id}` — includes `mechanics` (null if rejected)
- `POST /api/v1/evaluate` — request → room stats, morale rates, contributions, warnings
- `POST /api/v1/simulate` — request → totals, per-room and per-operator reports, morale trajectory, events, warnings
- `POST /api/v1/rosters`, `GET /api/v1/rosters`, `GET|PUT|DELETE /api/v1/rosters/{id}` — body `{ name?, roster }`, validated against the game data
- `POST /api/v1/bases`, `GET /api/v1/bases`, `GET|PUT|DELETE /api/v1/bases/{id}` — body `{ name?, base }`, validated
- `POST /api/v1/solves` — body `{ name?, request }`, answers 202 with the job; `GET /api/v1/solves` lists jobs with status; `GET|DELETE /api/v1/solves/{id}`

Invalid requests get `400` with a readable reason, for example
`invalid base: rooms draw 60 power but Power Plants supply 0`.

## Toolchain notes

- Rust stable, edition 2024. On Windows without MSVC the GNU host works;
  `ureq` is configured with `native-tls` so no C compiler is needed.
- `[profile.dev.package."*"] opt-level = 2` keeps the 16 MB JSON parse fast in
  test builds.
