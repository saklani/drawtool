# drawtool

Render Mermaid-style flowcharts as box diagrams in the terminal.

```
      ┌──────────┐
      │ load_env │
      └────┬─────┘
           ▼
       ╔════════╗
       ║ valid? ║
       ╚═══╤════╝
         ┌─┴──┐
         │    │
        yes   no
     ┌───┘    └──┐
     ▼           ▼
┌─────────┐   ┌──────┐
│ run_app │   │ exit │
└─────────┘   └──────┘
```

## Install

```sh
cargo install --path .
```

## Usage

```sh
drawtool diagram.mmd          # or pipe the source on stdin
drawtool --ascii diagram.mmd  # plain ASCII
drawtool -d LR diagram.mmd    # override direction (TD or LR)
```

## Syntax

A subset of Mermaid flowchart syntax:

```
flowchart TD                  %% TD = top-down (default), LR = left-to-right
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
