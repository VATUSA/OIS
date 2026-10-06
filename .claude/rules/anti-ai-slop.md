# Anti-AI-slop

Loads for every file. Governs word choice in anything a person reads: docs (`docs/`, `docs-site/`),
UI copy, issue and PR text, commit bodies, code comments. `engineering-prose.md` covers how
engineering prose should read; this file owns the banned lists, and that one does not repeat them.

Source: ported from the house anti-slop rule, minus its marketing material. Grounded in
[Wikipedia: Signs of AI writing](https://en.wikipedia.org/wiki/Wikipedia:Signs_of_AI_writing).

## Banned vocabulary

Cut these and say the plain thing. A word stays only when it has a specific technical meaning in
context (`dynamic` dispatch, a network `ecosystem` of crates).

- **Promotional verbs:** delve, showcase, underscore, foster, garner, enhance, navigate (figurative),
  embark, harness, empower, leverage, utilize, facilitate, commence, transform, revolutionize,
  unlock (figurative), elevate, streamline, supercharge.
- **Decorative adjectives:** robust, vibrant, intricate, multifaceted, comprehensive, holistic,
  seamless, bespoke, cutting-edge, paramount, pivotal, crucial, vital, noteworthy, profound,
  groundbreaking, a plethora of, a myriad of, a vast array of.
- **Buzzwords:** synergy, paradigm, journey (figurative), realm, landscape (figurative), beacon, game-changer,
  testament.
- **Hedges:** arguably, potentially, it's worth noting, it could be said, one might argue, could
  potentially.
- **Transitions:** furthermore, moreover, additionally. Fine inside a terse technical list, cut from
  running prose.

Grepping a draft for these takes seconds. Do it before you post or commit.

## Banned sentence patterns

| Pattern | Why it reads as generated |
| --- | --- |
| "It's not just X, it's Y" / "not because X but because Y" | Negative-parallelism template |
| "Whether you're X or Y" | Hollow hedging list |
| "From X to Y" used as a spacer | No information |
| "Imagine X" / "Picture this" | Intro with no point yet |
| "In today's fast-paced world" | Empty time anchor |
| "Designed to" / "Built to" | Avoids saying what it does |
| A reflexive triad ("faster, safer, simpler") | Rule of three on autopilot |
| "In this document we will…" / "In conclusion" | Meta-commentary |
| "The cause? A stale cache." | Rhetorical question, then answer |

## Banned structures

- Topic sentence, support, summary in every paragraph. Open some paragraphs with the fact, some
  with the example.
- Bullets that are all the same length, shape, and verb form. Uneven bullets read as honest.
- Treadmill paragraphs that say one thing twice in different words. Cut one.
- Ghost citations: "studies show", "it's well known". Name the source or drop the claim.
- Grandiose stakes. A config tweak does not "fundamentally reshape" metering.

## Punctuation

- **Em dashes:** use them sparingly in new prose. Most can become a comma, a period, or
  parentheses. Several per paragraph is a tell.
- **Straight quotes in source files.** Curly quotes belong only where a renderer produces them.

## Say the specific thing

- A number beats an adjective: "backs off to a 30 s poll when the feed goes stale"
  (`STALE_POLL_BACKOFF_SECS`, `backend/src/feed/mod.rs:66`) beats "smart polling". Read the
  number from code before you write it.
- Plain words beat Latinate ones: use, start, help, give, important.
- A 12-word sentence that commits beats a 22-word one that hedges.
- Name the symbol, file, or route: `feed/trajectory.rs`, `POST /api/v1/integration/jobs/lease`.

## Validate before claiming

Every factual claim in docs, UI copy, an issue, or a PR traces to code you read in this session.

- **Counts** (permissions, roles, enum values, migrations): count them in the file, don't
  estimate. Enum values come from the newest migration, see `database-postgres.md`.
- **Cadence and limits** (poll intervals, retention windows, rate limits): read the constant or
  the config. A default is not the configured value; see `AGENTS.md` § Working rules.
- **Behavior** ("OIS posts to Discord when…"): find the handler, job, or enqueue site that does
  it. For "X fires Y", grep every place Y is enqueued.
- **Existence** ("the bot supports slash commands"): grep for the implementation. `AGENTS.md`
  § Conventions & gotchas lists the subsystems that are designed but not built.

If you can't validate a claim, ask or leave it out.

## Self-audit before posting or committing prose

- [ ] Banned-vocabulary grep clean
- [ ] No "not X but Y", "Whether you're", or "From X to Y"
- [ ] Em dashes thinned
- [ ] Every claim traces to a file you read
- [ ] A number wherever a number exists
- [ ] American English (`behavior`, `color`, `-ize`); code identifiers and UI labels keep whatever
      the code uses
