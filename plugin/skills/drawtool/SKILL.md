---
name: drawtool
description: Sketch a quick box-and-arrow diagram in the terminal. Use when the user asks to explain or visualize a dataflow, control flow, call graph, pipeline, request lifecycle, state machine, or architecture ("explain the dataflow of this function", "how does X get to Y", "draw/diagram/sketch this"), or whenever a small diagram would explain code faster than prose.
---

# drawtool: terminal diagrams

`drawtool` turns a Mermaid-style flowchart into a Unicode box diagram and shows it to the user.

```bash
drawtool <<'EOF'
flowchart LR
req[request] --> parse(parse_body)
parse --> ok{valid?}
ok -->|yes| save[(db.insert)]
ok -->|no| err[400]
EOF
```

**The user sees the diagram where the Bash call ran.** This plugin's display mod draws a
`drawtool` call's output in full in the transcript, under a short `drawtool (flowchart LR, N
lines)` row. Don't copy the diagram into your reply: refer to it and explain it in a few
sentences. Run each diagram as its own Bash command that starts with `drawtool` (no pipes, no
`&&` chains), so the mod knows the call. If the user says they can't see it (Claude Code before
2.1.287 doesn't load mods), copy it verbatim into a plain ``` code block from then on.

## Input: standard Mermaid

Write standard Mermaid; drawtool reads it with Mermaid's own grammar. It draws:

- `flowchart LR` (preferred) or `flowchart TD`: dataflow, control flow, call graphs, pipelines,
  architecture. Use `subgraph` frames for layers or services, `{decision}` for branches,
  `-.->` for async or side effects, `==>` for the main path, `<-->` for request/response.
- `stateDiagram-v2` with `direction LR`: state machines and lifecycles (`[*]`, `<<choice>>`,
  `note`, composite `state X { }`).
- `sequenceDiagram`: call order between components (request → API → vendor → webhook → DB →
  client polls). Every message needs text (`a->>b: call`).

In every kind, a `%%` comment goes on its own line: after an edge it's an error, after a message
it becomes part of the label.

`RL`/`BT` are drawn as LR/TD. Not drawn yet: other kinds (class, ER, ...) and edges to a subgraph;
drawtool says so.
Shapes are drawn as the nearest of box, rounded, decision and database.

drawtool's own additions, which Mermaid reads as comments:

| Write | Meaning |
|---|---|
| `%% highlight: a, b, c` | draw that path thick (the branch a request takes, where a bug is) |
| `%% stack: save cache audit` | those boxes in one column, joined by short straight arrows: side effects (cache, log, metrics) under the step that causes them, or a pair that talks both ways (push/ack) instead of a line running back through the layers |

For "what does this screen look like" (UI code, a component tree, a design discussion), draw a
`wireframe`, drawtool's own format: `wireframe Title` then indented `header:`, `input:`,
`button:`, `card X:`, `row:`, `- item | $1` (see the README). Several screens side by side show a flow.

Flags: `-d TD` to override direction, `-w N` column budget (default 120 when run by Claude). A
too-wide LR diagram gets its labels wrapped, then continues on the next row, each edge crossing
the break drawn as a line wrapping around (several nest without crossing). drawtool never
switches to TD by itself; if it still can't fit, it prints a `note:` on stderr. Never paste that
note into the reply.
This plugin puts `drawtool` on PATH (it downloads the binary on first use). If that fails, it
says so on stderr; then use `~/.cargo/bin/drawtool`, or build it with
`cargo install --git https://github.com/saklani/drawtool`.

## Drawing a good diagram

- Show the real mechanism, read from the code: actual function names, data shapes on the edges
  (`-->|raw bytes|`, `-->|User dict|`). Don't invent steps.
- Keep it small: about 4 to 12 nodes. Collapse helpers into their caller; split a big flow into
  two diagrams instead of one huge one.
- Always use `flowchart LR` (or `direction LR` in a state diagram): it reads like a sentence and
  needs no scrolling.
- If a flow is too long for one line, break it yourself at a meaningful seam (request path,
  then processing, then storage) into several short LR diagrams, each with its own heading.
- If there is no clean seam, let drawtool wrap it into rows.
- TD is a mode, not a fallback: use `flowchart TD` only when the user asks for top-down.
- Keep labels short (< 25 chars); drawtool wraps them only when it has to.
- Use `{decision}` for branches, with edge labels on each branch (`yes`/`no`, `cache hit`/`miss`).
- If drawtool reports an error, fix the line it names and rerun.
