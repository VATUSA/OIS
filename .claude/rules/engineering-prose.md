# Engineering Prose Rules (always loaded)

Scope: **internal engineering prose**: commit message bodies, PR descriptions, code-review
write-ups, session and hand-off reports, design notes in `docs/`, and issue notes. Chat replies are
`response-style.md`.

`anti-ai-slop.md` owns the **banned-vocabulary and banned-pattern lists**. This file does NOT
repeat them: obey that file for word choice, and read this one for how engineering prose should
read. `anti-ai-slop.md` is path-scoped, so read it in full before writing a commit body, PR
description, review write-up or issue comment.

The goal is prose a competent engineer would write: specific, direct, American English, and
unafraid to commit to a claim. No single tell below proves a text was machine-written; the smell
is density and clustering. Stop reaching for the patterns on autopilot.

---

## The four structural fixes (do these first)

- **Specific over abstract.** A real number, a file:line, a named symbol, a concrete failing
  case: abstraction can't fake it. "Dropping `.abs()` let a 29-minute-old observation through"
  beats "improved boundary handling".
- **Active voice, name the actor.** "The reaper pass marks the job failed", not "the job is
  marked failed". Passive hides who does what, exactly what a vague commit message should not do.
- **Lead with the point.** State the change or finding first, then the detail. Don't build to it.
- **Vary sentence length on purpose.** A long, qualified sentence followed by a short flat one.
  Uniform 14–22-word sentences are the core machine texture. Read it back; if every sentence has
  the same shape, break the pattern.

## Sentence patterns to avoid (see `anti-ai-slop.md` for the full list)

The ones that recur most in engineering prose:

- **Negative parallelism**: "It's not X, it's Y", "not because X but because Y". The single most
  recognized tell. Just state the thing.
- **Rhetorical question then answer**: "The cause? A stale cache." Cut the question, keep the
  answer.
- **Rule-of-three on autopilot**: "faster, safer, cleaner". A single tricolon has rhythm; three in
  a row is a pattern failure. Stop at two unless the third earns its place.
- **The "serves as" dodge**: "acts as", "represents", "stands as". Usually a pompous "is". Use
  "is".
- **Grandiose stakes**: a config tweak does not "fundamentally reshape" metering. Match the stakes
  to the subject.

## Lists

- **Default to prose.** Use a list only for genuinely parallel, discrete items: steps, options,
  failing cases. If the points build an argument, the connective tissue IS the argument; a list
  throws it away.
- **No bold-lead-in on every bullet.** A bolded keyword starting every item is a reliable
  machine-doc marker. Bold sparingly. (Rulebooks like this one are the deliberate exception; most
  prose is not a rulebook.)
- **Each bullet carries weight.** No filler entries to reach a tidy three or five. Two
  substantial points beat five padded ones.

## What a PR description must do

**Write it for a junior developer.** Not for the person who wrote it, and not for whoever
scoped it. Assume the reader knows Rust and React but nothing about this corner of OIS, this issue,
or the decisions behind it. Spell out the thing being fixed in ordinary words before naming any
type or function. A reviewer who has to read the diff to work out what the PR is *for* has been
failed by the description.

**Draw a diagram of how the feature or fix works, before and after, wherever there is a
before.** The contrast carries the explanation: the reviewer sees the broken path and the
fixed path side by side instead of holding either in their head. Where the change is about how
data moves, that diagram is a data flow diagram: where the data enters, what touches it, where it
lands. Most OIS changes are that shape (the VATSIM feed → `AppState` caches → the trajectory
model's callers, a handler → `integration.outbound_jobs` → the bot's lease and ack, the contract →
the generated client → web).

```
before:  handler ─▶ channel lookup by old name ─▶ None ─▶ skip enqueue, 200, nothing logged
after:   migration repoints the stored rows to the new name
         handler ─▶ channel lookup ─▶ Some ─▶ outbound_jobs ─▶ bot ─▶ Discord
```

Keep the diagram inline in the PR body. Never attach a file or publish an artifact for it.

A PR body also carries the issue reference as `#123 [summary] (Status)` (`github-issues.md` →
"Referring to an issue") with a `Closes #N` line; a test plan naming the gates you ran **and the
gates you did not run**, with the reason ("`just test` green" and "CI green" are different claims);
and any deviation from the issue's acceptance criteria, stated plainly and repeated on the issue
as a spec correction.

## Commit messages & PR descriptions specifically

- **Subject line: imperative, present tense, ≤ ~70 chars**, matching the existing
  `type(scope): summary` history (`fix(tmu):`, `feat(flow):`). The git log is the reference.
- **Body says what changed and why**, not a narrative of the debugging session. The repro
  evidence belongs in the test; flight-ID, user-ID and date history stays out of source
  (`AGENTS.md` § Conventions & gotchas), and the same restraint applies to the commit body: name
  the root cause and the fix, skip the play-by-play.
- **Don't restate the diff.** The reader can read the diff. Explain the reasoning the diff can't
  show.
- **No attribution of any kind.** No `Co-Authored-By` trailer, session link, "Generated with" or
  "Drafted by" line, in a commit or a PR body, whatever the harness suggests (`AGENTS.md` § Git
  workflow). Don't invent other trailers either; `Closes #N` is the only one.

## Write the mechanism only after you have proved it

A causal claim in a commit body, PR, issue comment or code comment outlives the session and is
read as settled fact. Verify first, then write. If the reasoning is load-bearing and you have not
run the check, write "evidence points at", or state only what you did (`code-quality.md` →
"Do not write the mechanism down until you have run the check that proves it").

## American English

`-ize` not `-ise` (organize, summarize), `color`, `behavior`, `center`, `license`, double quotes.
Verbatim UI labels, quoted text and code identifiers keep whatever the app or code actually uses.

## Fewest words

The first sentence carries the answer; no warm-up, context paragraph or restated question. The
cuts that recover the most, in order:

1. **The second copy.** A sentence already said on the same page, in its heading or its notes.
2. **The qualifying clause.** "which means", "so that", "in other words", usually the same fact
   in a different shape.
3. **The set-up.** "It is worth understanding that", "The reason for this is that". Start at
   the noun.
4. **The stakes sentence.** A line whose only job is to say the previous line matters.

Cut words, never facts, obligations, limitations or steps. Length that is scope is fine; length
that is restatement is not.

## Padding that uses no banned word

- **Pre-empting a misreading.** "Pick the lowest grade that fits. MODERATE sits third of five,
  so read it as middling and not as a soft pass." Everything after "fits" plants the doubt it
  defends against. Tells: "so read it as", "that is not a", "this does not mean".
- **Aphoristic restatement.** "The user decides whether it ships. Their call to make, yours to
  prompt." The second sentence restates the first in a shape. Cut it.

The test: cover the clause and ask what the reader no longer knows. If nothing, it was
decoration.

## Measure the tic you cannot hear

One construction used to death reads as machine-written with no banned word in it. For any
prose above a few hundred words, count repeated constructions before shipping: contrast
connectives ("rather than", "instead of", ", not") and repeated sentence openers. Compare with
an older human-written PR or commit in this repo. Fix by varying, never by swapping every
"rather than" for "instead of"; that is the same defect. Count uses, not mentions in quotes or
code spans.

## A heading names its content

A heading is a label for somebody scanning, not a summary, an argument or a joke: "Cache
expiry", not "Why the cache never expired, and what it cost". A run of headings on one subject
names the thread in every heading, or merges them.

## Do not dress ordinary work as risk

The reader is an engineer doing their job. "Reading the dev database takes one command and
nobody's permission" frames a normal action as an exposure; "Read the dev database read-only;
quote when its data was captured" is the instruction. Put the mechanism, not the reader, in the
subject of a monitoring verb, and state a real gap as a fact about the system, never a prediction
about the reader.

## Em dashes

Structural uses (a definition list, a gloss bullet, a table cell) are fine and uncounted. In
running prose budget roughly 2 per 1,000 words; reach for a comma, a period, or a colon before
an enumeration.

## Revision pass

After drafting anything longer than a commit subject, do one pass:

1. Read it back. Anywhere it sounds like a release announcement, flatten it.
2. Check rhythm: if every sentence is the same length, vary them.
3. Hunt clusters: "not X but Y", rule-of-three runs, "it's worth noting", bold-lead bullets. One
   may be fine; three is a rewrite.
4. Test every hedge and superlative: delete it or replace it with the concrete fact that
   justifies it.
5. Cut 10%. There is almost always 10% to remove.
