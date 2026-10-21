# arknights-riic-planner

An accuracy-first optimiser for the Arknights RIIC (base). Rust workspace
backend, React frontend. See [`docs/plan.md`](docs/plan.md) for the full
design; this README tracks what actually exists.

## Status

| Layer | Crate | State |
| --- | --- | --- |
| 1. Data ingestion | `ak-data`, `ak-data-sync` | **Done.** Pinned upstream snapshot, schema drift check, strict two-stage transform. |
| 2. Skill DSL | `ak-data::mechanics` | **Done to 92% / 100%.** Description parser → typed `Mechanics` AST. 659 of 715 tiers fully modelled, 56 partial with named gaps, 0 rejected. |
| 3. Domain model | `ak-domain` | **Done.** `BaseConfig` (validated against room limits, layout slots and power), `Assignment` (checked mutators, so it cannot hold an impossible state), `Roster`. |
| 4. Evaluator / mood sim | `ak-eval` | **Done for production and morale.** Instantaneous evaluator plus a deterministic fixed-tick simulator with rotation, collection and a shared depot. Clue output and resource accumulators are not simulated yet. |
| 5. Solver | `ak-solver` | **Done.** Exhaustive search for small spaces, simulated annealing otherwise; a fast steady-state proxy inside the loop, the full simulator on the finalists; top-K with distinct scores; deterministic; progress reports and early stop through an `Observer`. |
| 6. Persistence | `ak-store` | **Done.** Versioned JSON documents for rosters, bases and solve jobs in Postgres, each owned by an account, migrated on read; pooled connections; schema migrations applied by `ak-api migrate`, so the server runs as a role that cannot change the schema. |
| 7. API | `ak-api` | **Done.** Game data, `evaluate` and `simulate`, stored rosters and bases that requests can name by id, and solves as a bounded background queue with live progress, stop / cancel, and recovery after a restart. Accounts with password sign-in and cookie sessions; everything but the game data, `/healthz` and signing up, in and out needs one, and each sees only its own documents. JSON errors throughout, per-client rate limits, sign-in backoff, per-account quotas, body and work limits, gzip, opt-in CORS. |
| 8. Roster import | `ak-data::import` | **Done.** Adapters for Krooster (public profile, current, row and legacy shapes; the API fetches a profile by username) and ak-planner exports, each tested against fixtures; unknown operators skipped and impossible promotions or levels clamped, with a report. |
| 9. Frontend | `frontend/` | **Done.** Vite + React + TS, no chart library: roster import with preview, a base editor on the in-game floor plan, a planning screen, live solve progress with stop, and results with finalists, a floor-plan diff, per-room output per day and morale sparklines; sign-in and account creation. |

**Ingestion coverage (pinned en_US snapshot):** 410 / 410 operators with
base skills, 715 / 715 skill tiers, 0 skipped under strict mode.

**Parser coverage (same snapshot):**

| Outcome | Tiers | Meaning |
| --- | --- | --- |
| Fully modelled | 659 (92.2%) | Every predicate, counter and effect is a typed AST node the evaluator can act on. |
| Partial | 56 (7.8%) | Parsed, but at least one part is an explicit `Unmodeled` node (e.g. "chance of higher-yield gold orders is increased"). The quantified parts are still usable. |
| Rejected | 0 | The parser could not read the description at all; the evaluator would report the skill and apply nothing. |

Every parsed magnitude that upstream also exposes through its display-only
`efficiency` sort hint agrees with that hint (over 200 tiers cross-checked
in `crates/ak-data/tests/mechanics.rs`). The coverage numbers are a test
ratchet: they may only go up, except when a new pin brings tiers the rules
do not know yet, and then the test says which.

## How Layer 2 works

Upstream ships **no** structured effect data. Each buff has an id, names,
icons, sort keys, a display-only `efficiency` percentage with `targets`, and
the localised description string. The mechanics live only in that string,
but it is highly templated, so the parser is compositional rather than one
rule per sentence:

1. **Templatize** the rich text: `<@cc.vup>+15%</>` → `{V:+15%}`, keywords
   → `{K:Guard}`, glossary terms → `{T:cc.g.bs}` (term ids are stable across
   locales; their display text is not: `cc.g.karlan` read "Karlan Trade" on
   Global until 2026, but its CN text, 谢拉格, shows it means the Kjerag
   nation, so that is what it maps to).
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
- **Who counts.** An operator counts when one of their skills took effect,
  when the model could not tell what one of their skills does, when another
  skill picked them out by name, faction, class or skill, or when they work
  in the Reception Room (whose rarity and promotion bonuses are not
  modelled). Anyone else working is filler: any operator without skills for
  the room would do the same there. A simulation reports, per operator, the
  rooms they were filler in.

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

A Factory, Trading Post or Office with nobody stationed does nothing; a
Power Plant still charges drones at its base rate (wiki.gg Factory, Trading
Post, Human Resources Office, Power Plant). An operator at zero morale
still counts as stationed: only their skills stop.

Working rooms drain 1.0 morale per hour. Factories and Trading Posts drain
0.05 less with two operators and 0.10 less with three, and every active
Control Center operator takes 0.05 per hour off every working operator.
Dormitories restore 1.5 + 0.1 × level + 0.0004 × ambience per hour.

`mood_threshold` rotation relieves a working operator at `swap_out` morale
with the best-rested substitute at or above `swap_in`: a bench operator,
or anyone resting in a Dormitory except its support (those whose skills
raise others' morale there after stacking, or whom another operator's
skill needs there), so the residents a plan puts in the Dormitories are
its second shift. The tired operator takes a free bed, or trades places
with a substitute resting in one, and trades back once rested to
`swap_in`. With no bed and no rested substitute, they work on.

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
no more Pure Gold or Shards than the starting stock plus what the Factories
make). Morale is priced as time. Without rotation an operator's contribution
stops when their morale would hit zero. Under rotation they work until
`swap_out` and then alternate work and rest, so their contribution is scaled
by `recovery / (recovery + drain)`, where `recovery` is the best Dormitory's
rate. The proxy takes about 0.2 ms; a 24-hour simulation about 40 ms. The
finalists are then re-scored by the full simulator and ranked by that, with
the best one's simulation returned in full. On the example bases the proxy
lands within 0.5% of the simulator.

**Objective.** A weighted sum over the horizon. Defaults are cost-based
where a cost exists and documented in `ak_solver::Objective`: LMD and EXP at
1 each, Orundum at 200 (what two Shards cost in LMD and Factory time),
drones at 20 (three minutes of Factory time each), and contacts, training,
unsold gold and exhausted hours at 0. Set your own.

**Results.** The top `top_k` finalists with distinct scores, each with its
breakdown; the starting assignment is always among them, so a solve can
never report something worse than what it was given. The same request and
seed give the same answer.

**Stopping early.** `time_budget_ms` ends either search after that long,
and an `Observer` passed to `solve_with` receives progress every 32
evaluations and can ask the search to end. Either way the finalists found
so far are still re-scored and returned, and `stopped` says why
(`time_budget` or `requested`). A run is otherwise never cut short: when
a proposal's random draws keep missing (with the whole pool placed,
replace and fill never apply), the annealer picks among the moves that
exist rather than giving up.

One consequence to know about: within a 24-hour horizon at full morale,
morale is free, so without rotation the solver will happily dismantle a
Control Center morale crew to put bodies in Factories. Enable rotation or
lengthen the horizon and the crew earns its keep: on the example base with
`mood_threshold` rotation over 72 hours, the solver keeps most of Team
Rainbow in the Control Center and lifts the score from 122 k to 225 k in
about two seconds (6 100 proxy evaluations, 7 simulations). The starting
assignment scores low because two of its Factories are unstaffed.

## How Layer 6 works

`ak-store` keeps rosters, bases and solve jobs as documents: an id, the
account that owns it, a name, timestamps, a `schema_version` and a JSON
body. Every read and write names the owner, so one account can never reach
another's documents, and an account's documents go when it does. Bodies are never
decomposed into columns. A document from an older schema is migrated on
read; one from a newer build is refused rather than guessed at.

Documents live in one Postgres table, `documents`, keyed by collection
and UUID, with the body as `jsonb`. The table comes from the SQL
migrations in `crates/ak-store/migrations/`, which are compiled into the
binary and applied by `ak-api migrate`, not by the server: the server
connects as a role that can read and write rows but not change the
schema, and refuses to start until every migration it knows has been
applied, unchanged. Queries are parameterised; nothing user-supplied is
ever spliced into SQL.

The server keeps a pool of connections (`--db-max-connections`, 10 by
default). A request that cannot get one within `--db-acquire-timeout-ms`
(3 seconds) is answered `503`, as is any request while the database is
down; idle connections close after ten minutes, and each is replaced
after thirty. See [Database](#database) for setting one up.

Solve documents and their life cycle are described under Layer 7.

## How Layer 7 works

`ak-api` is a library (`ak_api::router`, driven in-process by
`crates/ak-api/tests/api.rs`) plus a thin binary that connects to the
database, loads the data and serves it.

**References.** Any request that carries a `base` or `roster` (evaluate,
simulate, solve) may name a stored one instead, as `base_id` / `roster_id`.
References are resolved on arrival: a solve job stores the complete
request it ran, plus the ids under `refs`, so it stays reproducible if the
stored documents change.

**Rosters** arrive with a `source`: `manual` (the canonical shape, the
default), `krooster` or `ak-planner`. The last two run the Layer 8
adapters; the stored document keeps the canonical roster, its source and
the import report, and `POST /api/v1/rosters/preview` shows what an import
would store without storing it.

**Krooster by username.** `GET /api/v1/import/krooster/{username}` fetches
a user's public Krooster profile and answers with its roster alone,
`{ data: { roster } }`, ready to store with source `krooster`; the account
details and supports are dropped on arrival. The server fetches it because
Krooster sends no CORS headers, so the browser cannot. It is the only route
that reaches outside the server: the username must be one Krooster allows
(at most 32 letters, digits, `_` or `-`), the fetch gives up after 15
seconds, an unknown user is a `404`, and a Krooster failure is a `502`.
Answers are kept in memory, so Krooster is asked about a name at most once
in a while however many people fetch it: a roster for 5 minutes, served
with its age in seconds as `Age`, and an unknown user for 1 minute.
Failures are not kept. Requests for a name already being fetched wait for
that fetch, and at most 4 fetches run at once.

**Solves** are a queue. `POST /api/v1/solves` validates the request the way
the solver would, stores it as `pending` and answers `202`. At most
`--max-running-solves` run at once (half the cores by default); each runs on
a blocking thread and moves to `running`, then `done` or `failed`:

```text
pending ──▶ running ──▶ done      (result; result.stopped if stopped early)
   │           └──────▶ failed    (error)
   └──▶ cancelled                 (stopped before it started)
```

While a solve runs, `GET /api/v1/solves/{id}` includes `progress`: phase
(`searching` or `rescoring`), work done out of planned, evaluations, the
starting and best inner scores, and elapsed time. `POST
/api/v1/solves/{id}/stop` cancels a pending solve (`200`) or ends a running
one's search early and keeps what it found (`202`); a finished one is a
`409`. `DELETE` stops the solve and removes it, and nothing is written back
afterwards.

**Restarts.** On Ctrl-C the server tells running searches to stop and
leaves their documents as they are. On the next start every `pending` or
`running` solve is queued again and runs from the beginning; one that has
been cut off three times is marked `failed` instead. A solve that finishes
anyway (it was re-scoring, or had been stopped by request) is recorded
before the server exits.

**Limits.** `--max-ticks` (50 000 by default) bounds horizon ÷ tick for
simulate and solve requests, and `--max-solve-ms` (ten minutes) becomes the
time budget of any solve without a shorter one; the stored request shows
the budget that applied. Beyond that:

- *Per client.* Every client has a budget of requests that refills at a
  steady rate: 120 at once, then 20 a second (`--request-burst`,
  `--requests-per-minute`), across all routes. Signing up and signing in
  also draw on a much smaller one: 10 at once, then 5 a minute
  (`--sign-in-burst`, `--sign-ins-per-minute`). Over budget is `429`
  with `Retry-After`, before any other work. IPv6 clients are counted by
  their /64, the smallest block a network is given, so hopping between
  the addresses of one network buys nothing; a client holding a larger
  block gets one budget per /64, as one holding many IPv4 addresses gets
  one per address.
- *Who the client is.* The address the request connects from, unless that
  is a reverse proxy listed with `--trusted-proxy` (an address or a block
  such as `10.0.0.0/8`; none by default). Then the client is the
  rightmost `X-Forwarded-For` entry that is not itself a trusted proxy,
  the first address no client could have written; anything a client puts
  in the header is otherwise ignored. Behind a reverse proxy, list it:
  otherwise every client is the proxy, and all of them share one budget,
  the sign-in one included. The proxy must append to `X-Forwarded-For`
  (nginx: `proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;`);
  `X-Real-IP` alone is not read.
- *Per account.* At most 2 solves queued or running
  (`--max-solves-per-account`) and 2 simulations computing
  (`--max-simulations-per-account`: evaluate, simulate and a finalist's
  simulation) at once; over that is `429`. A simulation keeps its slot
  until it is done, even if its client has gone. A solve stopped or
  deleted while it waits gives its place back at once; a running one that
  is deleted keeps it until its search has wound down. Solves picked up
  after a restart were admitted before it and are not held back.
- *Bodies.* At most 1 MiB, and 16 KiB for signing up and in (`413`
  above that).

**Accounts.** `POST /api/v1/auth/register` creates an account from a
username (3 to 32 letters, digits, `_` or `-`, unique regardless of case),
a password and, optionally, an email address, and signs it in;
`/auth/login` signs in, `/auth/logout` signs out and `GET /auth/me` answers
with the account signed in.

- *Passwords* are 8 to 128 characters of anything, except the username,
  NFKC-normalised so the same password typed on different keyboards
  matches. They are stored as Argon2id hashes at OWASP's recommended cost
  (19 MiB, two passes); a hash made at an older cost is replaced at the
  next sign-in. Hashing runs on blocking threads, four at a time, and a
  hash keeps its slot until it is done even if its client has gone; a
  password over 4 KiB is refused before any of that work. A wrong
  password and an unknown username get the same `401` after the same
  work, so neither can be told apart.
- *Repeated failures.* Five wrong passwords in a row for one username are
  free; each after that makes the next attempt wait, 1 second at first and
  twice as long each time, up to a minute. An attempt during the wait is
  `429` with `Retry-After`, without its password being checked. No wait
  is longer than a minute, and the right password at the first attempt
  after one signs in and clears the count, as do 15 minutes without a
  failure; but the wait belongs to the username, so someone who keeps
  failing for it keeps it waiting. An attempt counts as a failure from the
  moment it is let through until its password proves right, so attempts
  sent all at once get no more than attempts sent one by one. Counts are
  kept by username whether or not an account has it, in memory, in a fixed
  table indexed by a secret hash (a restart forgets them).
- *The email* is checked only for shape and is not unique: it is not
  verified, so nobody can claim someone else's address first or learn
  which addresses have accounts. It is shown only to its owner.
- *Sessions* are 32 random bytes in a `__Host-session` cookie
  (`HttpOnly`, `Secure`, `SameSite=Lax`, `Path=/`), lasting 30 days.
  The database keeps only each token's SHA-256, so a leaked table cannot
  be replayed. Signing in always issues a fresh token and ends the one the
  request carried; signing out deletes it. Expired sessions are swept
  hourly. Browsers accept `Secure` cookies from `http://localhost` (Chrome,
  Edge and Firefox do; Safari does not), so the dev server works without
  HTTPS; anywhere else, serve over HTTPS.
- *Other sites.* `SameSite=Lax` keeps the cookie off other sites'
  fetches, frames and form posts; only a plain link, which can only
  `GET`, still carries it. On top of that, a request that would act on a
  session (any method but `GET`, `HEAD`, `OPTIONS` or `TRACE`, carrying
  the cookie) or start or end one (any such method under `/api/v1/auth/`)
  is refused with `403` when the browser's `Sec-Fetch-Site` header says
  it came from another site, which also covers another port on the same
  host. The cookie is matched by its exact name, so a look-alike such as
  `%5F%5FHost-session`, which another site could plant, is ignored. CORS
  never allows credentials, so accounts work only same-origin, through
  the dev proxy or a reverse proxy.
- Account responses are sent `Cache-Control: no-store`.

**Ownership.** The game data, `/healthz` and signing up, in and out are
open to everyone; every other route needs a signed-in account and answers
`401` without one. Each account lists, reads, changes, deletes and names
(as `base_id` / `roster_id`) only its own rosters, bases and solves;
another account's answers `404`, exactly as if it did not exist. Solves
run, are stopped and are written back as their owner's, and after a
restart each unfinished one is queued again for the account that submitted
it. These answers, like the account ones, are sent
`Cache-Control: no-store`.

**Errors** are always JSON, `{ "error": message }`, including malformed
bodies, unknown routes (`404`) and wrong methods (`405`, with `Allow`).
Anything wrong in a body is `400`, and shape errors name the path
(`base.rooms[0].kind: unknown variant …`), and a malformed document id is
`400` too. A missing `Content-Type` is `415` and an oversized body `413`.
A request that is not signed in, or a failed sign-in, is `401`; one
refused for coming from another site is `403`; one over a limit is `429`,
with `Retry-After` when the wait is known. Database failures are logged in
full and answered without detail: `503` when the database is unreachable
or busy, `500` otherwise.

**Transport.** Responses are gzip-compressed when the client accepts it (a
24-hour simulation shrinks from 41 KB to 4 KB). CORS is off unless
`--cors-origin` lists origins: the Vite dev server proxies `/api`, so the
frontend never needs it. CORS never carries credentials, so a listed
origin can use only the public game data and `/healthz`; everything that
needs an account works only same-origin.

## How Layer 8 works

`ak_data::import` turns another tool's roster export into the canonical
`Roster`. Each adapter reads one format only, so a change to one tool's
export breaks one adapter. Both tools key operators by the game's own
`char_…` ids, the same ids the pinned data uses, so nothing is matched by
name.

| Source | How to get the file | What is read |
| --- | --- | --- |
| `krooster` | Krooster has no export button, but profiles are public. Fetch one by username (the end of the profile link in Krooster's Settings, `krooster.com/u/…`) from the Rosters screen or `GET /api/v1/import/krooster/{username}`, or save the page `https://www.krooster.com/api/u/{username}`. Last resort, in a browser logged in to Krooster: open Data → Collection, then run `copy(localStorage.getItem("v3_roster"))` in the console (Chrome asks you to type `allow pasting` first). | Profile: `{ data: { account, supports, roster } }`, of which only `roster` is read, in the current shape: `{ id: { op_id, elite, level, potential, … } }`. The current shape is also accepted bare (`v3_roster`), as a list of rows (its database table), and in its older `{ id: { id, promotion, owned, level, … } }` shape. |
| `ak-planner` | [GoodEffort/Arknights-Planner](https://goodeffort.github.io/Arknights-Planner/): "Import/Export" at the top right, copy the text. | `{ s, i, p }`: each saved plan's `operatorId`, `plans.currentElite` and `plans.currentLevel`, in its current and older shapes. |

The formats come from each tool's source code (`neeia/ak-roster`:
`src/types/operators/operator.ts`, `src/util/hooks/useOperators.ts`,
`src/pages/api/u/[user].tsx`;
`GoodEffort/Arknights-Planner`: `src/store/store-operator-functions.ts`,
`src/types/plans.ts`), and the fixtures in
`crates/ak-data/tests/fixtures/import/` follow them rather than real
accounts. The plan called the second tool "ak-planner (aceship's tool)";
no Aceship tool exports a roster, and GoodEffort's planner is the one whose
code calls itself `akplanner`.

Every operator is checked against the pinned data, and the report says
what changed:

- an id the data lacks (released after the snapshot, CN-only, Amiya's
  alternate forms, no base skills) is skipped with `unknown_operator`;
- a promotion the rarity cannot reach (a 3★ at Elite 2) is taken as fully
  raised at the highest promotion it can reach (`promotion_clamped`); a
  level outside `1..=max` for the promotion is clamped (`level_clamped`),
  using `phases[].maxLevel` from the character table;
- a repeated operator keeps its last entry (`duplicate`);
- an ak-planner operator selected without a saved plan is taken as Elite 0
  level 1, as ak-planner shows it (`no_saved_plan`).

A file in the wrong format, or an entry with a field of the wrong type, is
refused with the entry named
(``char_102_texas: `elite` must be 0, 1 or 2, found 3``). Handing one
tool's file to the other adapter says which tool it looks like.

**Gaps, and the defaults chosen:**

- Neither tool records current morale, so every imported entry has
  `mood: null` and the simulation's `initial_mood` policy decides it.
- ak-planner is a planner, not a roster: it lists only the operators you
  are planning upgrades for, with no notion of ownership. Every one of them
  is imported as owned at its *current* promotion, inactive plans included.
- Krooster marks ownership by keeping an entry at potential 1 or more; its
  older shape is also skipped when `owned` is `false`.
- Skill levels, masteries, modules, potential and skins are read past: base
  skills depend only on promotion and level.

```bash
cargo run -p ak-cli -- import krooster roster.json                  # summary and warnings
cargo run -p ak-cli -- import ak-planner plan.json --out roster.json  # write the canonical roster
```

## How Layer 9 works

The frontend (`frontend/`, Vite + React + TypeScript, no router or chart
library) has one screen per step, switched by the URL hash. Rosters, Base,
Plan and Results belong to an account: while nobody is signed in they show
a sign-in card instead, which can also create an account. The header shows
who is signed in, with a Sign out button, and a sign-in or sign-out in
another tab is followed at once. If a session instead ends under an open
screen (it expired), the next request brings back the sign-in card, and
the screen stays behind it with anything unsaved until the same account
signs in again; another account gets fresh screens. Remembered picks (the
last roster and base) are forgotten when a different account signs in.

- **Rosters** (`#/rosters`): fetch a Krooster roster by username (one the
  server kept from a minute or more ago says how old it is), or paste
  or open another tool's export (or the canonical JSON), preview what it
  reads as, with every warning named, and save it. A stored roster shows its
  import report and its operators by rarity, filtered by name or promotion.
- **Base** (`#/base`): rooms with their level, Factory formula, Trading Post
  orders, Dormitory ambience and Training Room job; running totals of power
  and of production, dormitory and function slots, from the same data the
  server validates with; and the base on the in-game floor plan
  (`building_data.layouts.v0`), each room in a slot of its category and
  size.
- **Plan** (`#/plan`): a stored base and roster, the Pure Gold already in
  the depot (a slider or an exact number), the horizon (24 h, 72 h, a week),
  rotation thresholds, the objective weights with their defaults,
  operators pinned to rooms (sent as the starting assignment plus locked
  slots), and an effort preset (Quick, Standard, Thorough) that sets
  iterations, restarts and the time budget. It defaults to 72 hours with
  rotation, where morale relief has a price.
- **Results** (`#/results/{id}`): live progress with a Stop button while the
  solve runs; then per-day tiles against the start, the finalists with their
  trade-offs, the chosen finalist on the floor plan with changes against the
  best (or the start) outlined and listed, with operators who were filler
  in their room shown as "Filler unit" there and in the tables below (one
  filler swapped for another is no change against the best), output per
  room per day with where the Pure Gold went and whether the posts ran
  short of it, morale
  as one sparkline per operator grouped by room with the lowest reading and
  any hours exhausted (hover or arrow keys for a reading; exhaustion is
  marked with a dot and the hours), or as a table with hours working and
  resting, and everything the model could not honour. Relief that never
  came on is named in one line rather than charted. Finalists other than
  the best are simulated on request, the way the solve scored them.
- **Game data** (`#/data`): provenance, every operator, and the raw
  simulate and solve panels for developers. The panels need a signed-in
  account, since requests run on one; the rest is public.

The chart colours are the first three slots of a palette validated for
colour-blind separation in light and dark mode; every coloured thing also
carries a text label. The theme follows the system unless the switch in the
header picks light or dark, which the browser remembers. Item names for
Factory formulas are a display table in `frontend/src/format.ts`, since the
item table is not ingested.

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
    src/import/         Layer 8: Krooster and ak-planner roster adapters
    tests/fixtures/import/  export fixtures for the adapters
  ak-data-sync/         fetch pinned SHA, verify digests, validate
  ak-eval/              Layer 4: evaluator + mood simulator (pure, no I/O)
    src/rules.rs        every rule not in the data, with its source
    tests/scenarios.rs  golden scenarios, one per rule
  ak-solver/            Layer 5: space, exhaustive, anneal, objective, evaluator
    tests/solver.rs     exhaustive vs annealing, determinism, locks, budget
  ak-store/             Layer 6: versioned JSON documents in Postgres
    migrations/         the schema, applied in order by `ak-api migrate`
  ak-api/               Layer 7: axum routes, references, job queue, limits
    src/auth/           accounts: password hashing, session cookies, cross-site guard, sign-in backoff
    tests/api.rs        the router driven in-process
  ak-cli/               `ak stats | op | skill | find | skipped | mechanics | simulate | solve | import`
examples/requests/      simulation and solve requests for a 2-4-3 base
frontend/               Layer 9: Vite + React + TypeScript
  src/views/            Rosters, Base, Plan, Results, Game data screens
  src/components/       BaseMap (floor plan SVG), MoraleTable (sparklines),
                        ui (page header, segmented control, meter), icons
  src/dev/              raw simulate and solve panels
```

Dependency direction is strictly `ak-domain ← ak-data ← {sync, cli, api}`,
`ak-domain ← ak-eval ← ak-solver ← {cli, api}` and `ak-store ← ak-api`.
`ak-eval` and `ak-solver` must never grow an async, I/O or framework
dependency.

## Upstream data

Source: [ArknightsAssets/ArknightsGamedata](https://github.com/ArknightsAssets/ArknightsGamedata),
`en`, commit `09df1ef62bb5` (the EN update of 2026-09-23, resource version
`26-09-18-15-44-33_adcee5`). It extracts the data from the game's servers
with [arkprts](https://github.com/thesadru/arkprts) in GitHub Actions and is
the source Krooster uses for Global. It writes an empty list as `{}` rather
than `[]`; the raw mirrors accept both.

The previous source, [Kengxxiao/ArknightsGameData_YoStar](https://github.com/Kengxxiao/ArknightsGameData_YoStar),
was archived on 2025-11-13 (client 31.5.80), so operators released to Global
after that (Haruka, Hoshiguma the Breacher, the Ave Mujica collab, …) were
missing and imports skipped them. The CN repository is still live and is
listed (disabled) in the manifest with the same schema. Note that the Layer 2
parser is written against English descriptions; a CN source would need a
second rule table (term ids and values are shared, prose is not).

Files ingested: `building_data.json`, `character_table.json`,
`handbook_team_table.json` (~25 MB, committed).

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
cargo test --workspace                     # the ak-store and ak-api tests need Postgres (below)
cargo run -p ak-data-sync -- check         # verify pins, digests, schema, transform
cargo run -p ak-cli -- stats               # counts + parser coverage as JSON
cargo run -p ak-cli -- op char_285_medic2  # one operator, resolved skills
cargo run -p ak-cli -- simulate examples/requests/243-base.json             # 24 h report
cargo run -p ak-cli -- simulate --evaluate examples/requests/243-base.json  # starting instant
cargo run -p ak-cli -- solve examples/requests/solve-243.json               # search, ~2 s
cargo run -p ak-cli -- import krooster roster.json                          # read another tool's roster
cargo run -p ak-api -- migrate             # create or update the database schema
cargo run -p ak-api                        # http://127.0.0.1:8080
cargo run -p ak-api -- --help              # database pool, limits, CORS origins, data path
```

```bash
cd frontend && npm install && npm run dev  # http://localhost:5173, proxies /api to the backend
```

### Database

The API needs Postgres 15 or later. Three roles keep the server away
from the schema: `riic_owner` owns it and is used only to migrate,
`riic_app` is what the server connects as and may only read and write
rows, and `riic_test` creates a throwaway database per test. Once, as the
`postgres` superuser (`psql -U postgres`), with passwords of your own:

```sql
CREATE ROLE riic_owner LOGIN PASSWORD '…';
CREATE ROLE riic_app LOGIN PASSWORD '…';
CREATE ROLE riic_test LOGIN CREATEDB PASSWORD '…';
CREATE DATABASE riic OWNER riic_owner;
CREATE DATABASE riic_test OWNER riic_test;
ALTER ROLE riic_app SET statement_timeout = '30s';
\connect riic
REVOKE ALL ON SCHEMA public FROM PUBLIC;
GRANT USAGE ON SCHEMA public TO riic_app;
ALTER DEFAULT PRIVILEGES FOR ROLE riic_owner IN SCHEMA public
    GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO riic_app;
```

Then copy `.env.example` to `.env` (git-ignored), fill in the passwords,
and run `cargo run -p ak-api -- migrate`. The binary and the tests read
`.env` from the working directory or above it; real environment
variables win over it. Keep database URLs out of flags, which other users
of the machine can see, and add `?sslmode=verify-full` to any URL that
leaves the machine.

API endpoints. All but the game data, `/healthz` and signing up, in or out need a signed-in account (`401` without one), and act only on that account's documents:

- `GET /healthz`
- `POST /api/v1/auth/register` — body `{ username, password, email? }`, answers `201` with the account and a session cookie, `409` if the username is taken
- `POST /api/v1/auth/login` — body `{ username, password }`, answers the account and a session cookie, `429` while the username must wait after failed attempts
- `POST /api/v1/auth/logout` — ends the session, `204`
- `GET /api/v1/auth/me` — the account signed in, or `401`
- `GET /api/v1/gamedata/version` — source, SHA, fetch time, parser version, counts, coverage
- `GET /api/v1/gamedata/operators` — summary list
- `GET /api/v1/gamedata/operators/{id}`
- `GET /api/v1/gamedata/skills/{id}` — includes `mechanics` (null if rejected)
- `GET /api/v1/gamedata/facilities` — room kinds with per-level capacity, power and build cost
- `GET /api/v1/gamedata/formulas` — Factory formulas
- `GET /api/v1/gamedata/layout` — the base's slots with grid positions
- `GET /api/v1/gamedata/constants` — global tuning values (Dormitory ambience limit, …)
- `POST /api/v1/evaluate` — request → room stats, morale rates, contributions, warnings
- `POST /api/v1/simulate` — request → totals, per-room and per-operator reports, morale trajectory, events, warnings
- `POST /api/v1/rosters`, `GET /api/v1/rosters`, `GET|PUT|DELETE /api/v1/rosters/{id}` — body `{ name?, source?, roster }`, where `source` is `manual` (default), `krooster` or `ak-planner`; answers with the import report
- `POST /api/v1/rosters/preview` — same body; the roster and import report, not stored
- `GET /api/v1/import/krooster/{username}` — the roster in a Krooster user's public profile, `{ data: { roster } }`, to store with source `krooster`; kept for a few minutes, with its age in `Age`
- `POST /api/v1/bases`, `GET /api/v1/bases`, `GET|PUT|DELETE /api/v1/bases/{id}` — body `{ name?, base }`, validated
- `POST /api/v1/solves` — body `{ name?, request }`, answers `202` with the job, `429` if the account already has as many queued or running as it may; `GET /api/v1/solves` lists jobs with status
- `GET /api/v1/solves/{id}` — status, the request as it ran, `progress` while running, the result or error once finished
- `POST /api/v1/solves/{id}/stop` — cancel if pending, end the search early if running
- `DELETE /api/v1/solves/{id}` — stop if needed, then remove
- `GET /api/v1/solves/{id}/candidates/{n}/simulation` — finalist `n` (0 is the best) simulated the way the solve scored it

Evaluate, simulate and a finalist's simulation answer `429` while the
account already has as many simulations computing as it may.

Evaluate, simulate and solve requests may use `base_id` / `roster_id` in
place of `base` / `roster`. Invalid requests get `400` with a readable
reason, for example `invalid base: rooms draw 60 power but Power Plants
supply 0`.

## Toolchain notes

- Rust stable, edition 2024. On Windows without MSVC the GNU host works;
  `ureq` and `sqlx` are configured with `native-tls` so no C compiler is
  needed. That also rules out SQLite, whose crates compile it from C.
- `[profile.dev.package."*"] opt-level = 2` keeps the 16 MB JSON parse fast in
  test builds.
