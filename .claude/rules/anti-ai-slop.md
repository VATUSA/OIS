---
paths:
  - "docs/**"
  - "docs-site/**"
  - "README.md"
  - "CONTRIBUTING.md"
  - "web/src/pages/**/*.tsx"
  - "web/src/components/**/*.tsx"
  - "web/src/features/**/*.tsx"
  - "packages/ui/src/**/*.tsx"
---
# Anti-AI-Slop Rules (not loaded at startup; read in full before writing docs or UI copy, a commit body, a PR description or an issue comment)

Governs word choice in anything a person reads: design docs (`docs/`), the user docs site
(`docs-site/`), UI copy, issue and PR text, commit bodies and code comments. `engineering-prose.md`
covers how engineering prose should read; this file owns the banned lists, and that one does not
repeat them.

Generated text is identifiable to many readers, and once a reader spots it they trust the rest of
the page less. This rule captures what to ban, what to prefer, and how to validate every claim.

Source: [Wikipedia: Signs of AI writing](https://en.wikipedia.org/wiki/Wikipedia:Signs_of_AI_writing).

---

## Banned vocabulary — high-signal AI tells

These words don't appear in OIS prose unless there's a specific technical reason (`dynamic`
dispatch, a crate `ecosystem`). Default position is: cut and replace.

**Verbs/promotional**:
delve, showcase, underscore, foster, fostering, garner, enhance,
enhanced, navigate (figurative), navigating, embark, embarking, harness,
harnessing, empower, empowering, leverage, utilize, facilitate,
commence, transform, transformative, revolutionize, revolutionary,
unlock (when figurative), elevate, streamline, supercharge

**Adjectives/decorative**:
robust, vibrant, intricate, multifaceted, comprehensive, holistic,
seamless, seamlessly, bespoke, cutting-edge, paramount, pivotal,
crucial, vital, noteworthy, commendable, dynamic, profound,
groundbreaking, plethora (of), myriad, vast array

**Buzzwords**:
synergy, paradigm, ecosystem (when not technical), tapestry, realm,
landscape (figurative), beacon, game-changer, game-changing,
journey (figurative), testament

**Hedges**:
arguably, potentially, it's worth noting, it could be said,
one might argue, in certain contexts, could potentially be

**Transitions** (cut almost all):
furthermore, moreover, additionally (in body prose; fine in
technical bullet lists)

A grep of a draft for any of these words takes seconds. Do it before you post or commit.

## Banned sentence patterns

| Pattern | Why it's slop |
| --- | --- |
| "It's not just X — it's Y" / "not because X but because Y" | Negative-parallelism template, model favorite |
| "Whether you're X or Y" | Hedging-list template, almost always hollow |
| "From X to Y" (especially repeated) | Spacer phrase, no information density |
| "Imagine X" / "Picture this" | Filler intro, no point made yet |
| "In today's [adj] world/era/landscape" | Empty time-anchor |
| "At the end of the day" | Filler |
| "Designed to / Built to" | Avoids saying what it actually does |
| Triadic lists ("faster, smarter, simpler") | Rule of three, model favorite |
| "Built from the ground up" | Cliché, no information |
| "In this document, we will..." | Meta-commentary |
| "In conclusion / To wrap up" | Meta-commentary |
| "The cause? A stale cache." | Rhetorical question, then answer |

## Banned structural patterns

- **Topic sentence → support → summary in every paragraph**. Vary structure: one paragraph might
  open with the fact, another with the example.
- **Perfect parallel construction across bullets** (every bullet same length, same shape, same
  verb form). Bullets are allowed to be uneven.
- **"Despite [positives], [subject] faces challenges..."** formula. Skip the formula; state the
  challenge directly.
- **Treadmill paragraphs**: paragraphs that say something twice in different words. Cut one.
- **Ghost citations**: "studies show", "experts agree", "it's well known" with no source. Either
  name the source or don't make the claim.
- **Grandiose stakes**: a config tweak does not "fundamentally reshape" metering.

## Punctuation

- **Em dashes**: aim for ≤ 1 per ~500 words of body prose. Replace most em dashes with commas,
  periods, or parentheses. Several per paragraph is a tell.
- **Curly quotes and apostrophes** in source files: use straight quotes in Rust, TypeScript and
  Markdown source. Curly versions belong only where a renderer produces them.
- **Triple-bullet rule of three** is a slop tell when applied uniformly. A list of four with one
  odd item is more honest than a list of three perfectly balanced items.

## Voice principles — what to do instead

- **Specific over abstract**: "backs off to a 30 s poll when the feed goes stale"
  (`STALE_POLL_BACKOFF_SECS`, `backend/src/feed/mod.rs`) beats "smart polling". Read the number from
  code before you write it.
- **Concrete over corporate**: "FCA metering times the descent on its own speed schedule" beats
  "improved ETA accuracy".
- **Plain over Latinate**: "use" not "utilize", "start" not "commence", "make easier" not
  "facilitate", "important" not "paramount", "help" not "empower", "give" not "provide".
- **Short over long**: a 12-word sentence that commits beats the 22-word version that hedges. Cut
  the hedges, keep the fact.
- **Show numbers, not adjectives**: a count read from the file is evidence; "extensive coverage" is
  decoration.
- **Name the symbol, file or route**: `feed/trajectory.rs`, `POST /api/v1/integration/jobs/lease`.

## Validate before claiming

Every factual claim in docs, UI copy, an issue or a PR traces to code you read in this session. If
you can't find a code source for a claim, do not write it. Specifically:

- **Counts** (permissions, roles, enum values, migrations): count them in the file, don't
  estimate. Enum values come from the newest migration (`database-postgres.md`).
- **Cadence and limits** (poll intervals, retention windows, rate limits): read the constant or
  the config. A default is not the configured value (`code-quality.md` → "A config default is not
  the configured value").
- **Behavior claims**: "OIS posts to Discord when…" requires a handler, job or enqueue site that
  does it. For "X fires Y", grep every place Y is enqueued.
- **Feature-existence claims**: grep for the implementation before stating it exists. Some
  subsystems are designed but not built (`AGENTS.md` § Conventions & gotchas): transactional email
  is design-only, and the Discord bot registers no slash commands.

If you genuinely can't validate a claim, **ask the user**. Don't make it up.

## Self-audit checklist before commit

- [ ] Em dashes counted; under budget?
- [ ] Banned-vocabulary grep clean?
- [ ] Every claim traces to a code source read this session?
- [ ] No "Whether you're..." / "From X to Y" / "It's not just..."?
- [ ] Specific numbers wherever a claim could carry one?
- [ ] No corporate-buzzword filler?
- [ ] American English (`behavior`, `color`, `-ize`); code identifiers and UI labels keep the code's
      spelling?
- [ ] If a claim couldn't be validated, asked rather than invented?
