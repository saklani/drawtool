---
name: drawtool
description: Sketch a quick box-and-arrow diagram in the terminal. Use when the user asks to explain or visualize a dataflow, control flow, call graph, pipeline, request lifecycle, state machine, or architecture ("explain the dataflow of this function", "how does X get to Y", "draw/diagram/sketch this"), or whenever a small diagram would explain code faster than prose.
---

# drawtool: terminal diagrams

`drawtool` turns a Mermaid-style flowchart into a Unicode box diagram you can paste into your reply.

```bash
drawtool <<'EOF'
flowchart LR
req[request] --> parse(parse_body)
parse --> ok{valid?}
ok -->|yes| save[(db.insert)]
ok -->|no| err[400]
EOF
```

**The user does not reliably see Bash output.** Copy the rendered diagram verbatim into your
reply inside a plain ``` code block, then explain it in a few sentences.

## Syntax

| Write | Meaning |
|---|---|
| `flowchart LR` / `flowchart TD` | left-to-right (preferred) / top-down |
| `a[text]` `a(text)` `a{text}` | box / rounded (function, process) / decision (double border) |
| `a --> b`, `a --> b --> c` | arrow, chains |
| `a -->\|label\| b` or `a -- label --> b` | labelled edge (what data flows) |
| `a -.-> b` / `a ==> b` / `a --- b` | dotted (async, optional, side effect) / thick (main path) / no arrow |
| `a --> b & c`, `a & b --> c` | fan-out / fan-in |
| `a[line one<br>line two]` | multi-line label |
| `wireframe Title` then indented `header:`, `input:`, `button:`, `card X:`, `row:`, `- item \| $1` | screen mockup (see README) |
| `subgraph api [API server]` ... `end` | dashed group frame (layers, services, vendors) |
| `%% highlight: a, b, c` | draw that path thick (the branch a request takes, where a bug is) |
| `%% comment` | comment |
| `sequenceDiagram` then `a->>b: msg`, `b-->>a: reply` | sequence diagram (see below) |

Define a node's label once (`p(parse_body)`), then refer to it by id (`p`); if a label is given twice, the last
one wins (as in Mermaid). Cycles and self-loops work. The input is a strict subset of Mermaid, so
the same source renders on GitHub; `RL`/`BT`, `<-->`, `--o`/`--x`, `~~~` and edges to a subgraph are errors.
For "what does this screen look like" (UI code, a component tree, a design discussion), draw a
`wireframe` from the real components and copy; several screens side by side show a flow.
`drawtool --diff before.mmd after.mmd` draws one diagram of a change: `+` added, `-` removed,
`~` relabelled. Use it to explain a refactor or a PR (write both versions to temp files).
Flags: `--ascii` for plain ASCII, `-d TD` to override direction, `-w N` column budget
(default 120 when run by Claude). A too-wide LR diagram gets its labels wrapped, then continues on
the next row: a single crossing edge is drawn wrapping around, several are joined by numbered
connectors (①). drawtool never switches to TD by itself; if it still can't
fit, it prints a `note:` on stderr. Never paste that note into the reply.
If `drawtool` isn't on PATH, use `~/.cargo/bin/drawtool`, or build it with
`cargo install --path /Users/shauryasaklani/Development/drawtool`.

**Sequence diagrams:** when the question is about call order between components (request → API
→ vendor → webhook → DB → client polls), start with `sequenceDiagram` instead of a flowchart.
Messages: `a->>b: text` (call), `b-->>a: text` (reply), `a->>a: text` (self), `a--xb: 400`
(failure). Also `participant db as Postgres`, `actor user`, `Note over a,b: text`, and frames
`loop` / `alt` + `else` / `opt` / `par` + `and`, each closed by `end`.

## Drawing a good diagram

- Show the real mechanism, read from the code: actual function names, data shapes on the edges
  (`-->|raw bytes|`, `-->|User dict|`). Don't invent steps.
- Keep it small: about 4 to 12 nodes. Collapse helpers into their caller; split a big flow into
  two diagrams instead of one huge one.
- Always use `flowchart LR`: it reads like a sentence and needs no scrolling.
- If a flow is too long for one line, break it yourself at a meaningful seam (request path,
  then processing, then storage) into several short LR diagrams, each with its own heading.
- If there is no clean seam, let drawtool wrap it into bands.
- TD is a mode, not a fallback: use `flowchart TD` only when the user asks for top-down.
- Keep labels short (< 25 chars); drawtool wraps them only when it has to.
- Use `{decision}` for branches, with edge labels on each branch (`yes`/`no`, `cache hit`/`miss`).
- If drawtool reports a parse error, fix the line it names and rerun.
