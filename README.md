# drawtool

Render Mermaid-style flowcharts as box diagrams in the terminal. Left-to-right by default,
so a diagram reads like a sentence and fits on screen without scrolling.

```
                                                  ╭─────────────╮
                                                ┌►│ dead letter │
┌───────────┐  ╭────────╮  ╔════════════╗ ┌─no──┘ ╰─────────────╯
│ raw bytes ├─►│ decode ├─►║ schema ok? ╟─┤
└───────────┘  ╰────────╯  ╚════════════╝ └─yes─┐ ┌─────────────┐  ╭───────────╮
                                                └►│ transform ↻ ├─►│ warehouse │
                                                  └─────────────┘  ╰───────────╯
```

```
flowchart LR
in[raw bytes] --> dec(decode) --> v{schema ok?}
v -- no --> dlq[(dead letter)]
v -- yes --> t[transform] --> out[(warehouse)]
t --> t
```

## Install

```sh
cargo install --path .
```

## Usage

```sh
drawtool diagram.mmd          # or pipe the source on stdin
drawtool --ascii diagram.mmd  # plain ASCII
drawtool -d TD diagram.mmd    # override direction (LR or TD)
drawtool -w 100 diagram.mmd   # column budget (default $COLUMNS or 120, 0 = unlimited)
```

## Long diagrams

Diagrams are fitted to the column budget. A left-to-right diagram that is too wide is,
in order of preference:

1. drawn with long labels word-wrapped,
2. broken into stacked bands joined by numbered connectors (①, ②, ...),
3. drawn top-down, as a last resort (drawtool prints a `note:` on stderr).

```
┌──────────────┐  ╭───────╮                ╭──────────────╮  ╭───────────────╮  ╭────────────────────╮  ╭───╮
│ diagram text ├─►│ parse ├──nodes, edges─►│ break_cycles ├─►│   layering    ├─►│ crossing reduction ├─►│ ① │
└──────────────┘  ╰───────╯                ╰──────────────╯  │ + dummy nodes │  ╰────────────────────╯  ╰───╯
                                                             ╰───────────────╯

╭───╮  ╭─────────────╮  ╭───────────────────────╮  ╭───────────╮       ┌────────┐
│ ① ├─►│ place: PAVA ├─►│ ports + track routing ├─►│ draw grid ├──str─►│ stdout │
╰───╯  ╰─────────────╯  ╰───────────────────────╯  ╰───────────╯       └────────┘
```

```
flowchart LR
src[diagram text] --> parse(parse) -->|nodes, edges| cyc(break_cycles)
cyc --> lay(layering<br>+ dummy nodes) --> ord(crossing reduction) --> pos(place: PAVA)
pos --> route(ports + track routing) --> draw(draw grid) -->|str| out[stdout]
```

## Syntax

A subset of Mermaid flowchart syntax:

```
flowchart LR                  %% LR = left-to-right (default), TD = top-down
req[HTTP request] --> parse(parse_body)
parse --> ok{valid?}
ok -->|yes| save[(db.insert)]
ok -- no --> err[400 Bad Request]
save -.-> cache[invalidate]   %% dotted
save ==> resp[201 Created]    %% thick
a --> b & c                   %% fan-out (also: a & b --> c)
x --- y                       %% no arrowhead
n[two\nlines]                 %% multi-line label (\n or <br>)
```

Shapes: `[box]`, `(rounded)`, `{decision}`, `([stadium])`, `[(database)]`, `[[sub]]`.
Cycles and self-loops are supported. `subgraph` blocks are not rendered (their lines are ignored).
Unlike Mermaid, a diagram without a `flowchart` line is drawn left-to-right.

## Claude Code skill

`skills/drawtool/SKILL.md` teaches Claude Code to sketch diagrams with drawtool when you ask it
to explain a dataflow or control flow. Link it into your skills directory:

```sh
ln -s "$PWD/skills/drawtool" ~/.claude/skills/drawtool
```

## Tests

```sh
cargo test                    # examples/*.mmd must match tests/golden/
UPDATE_GOLDEN=1 cargo test    # accept intentional rendering changes
```

## License

MIT
