# drawtool

Render Mermaid-style flowcharts as box diagrams in the terminal. Left-to-right by default,
so a diagram reads like a sentence and fits on screen without scrolling.

```
                                          ┌─────┐
                                        ┌►│ 401 │
┌────────────────┐  ╔═══════════╗ ┌─no──┘ └─────┘
│ GET /users/:id ├─►║ token ok? ╟─┤
└────────────────┘  ╚═══════════╝ └─yes─┐ ╔═════════╗ ┌─yes─────────────────────────────────────┐ ┌────────┐
                                        └►║ cached? ╟─┤                                         ├►│ 200 OK │
                                          ╚═════════╝ └─no──┐ ╭──────────╮       ╭────────────╮ │ └────────┘
                                                            └►│ postgres ├──row─►│ fill cache ├─┘
                                                              ╰──────────╯       ╰────────────╯
```

```
flowchart LR
req[GET /users/:id] --> auth{token ok?}
auth -->|no| err[401]
auth -->|yes| hit{cached?}
hit -->|yes| ok[200 OK]
hit -->|no| db[(postgres)] -->|row| fill(fill cache) --> ok
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
drawtool -w 100 diagram.mmd   # column budget (default: terminal width, 0 = unlimited)
```

## Long diagrams

Diagrams are fitted to your terminal (`$COLUMNS`, else the terminal's width, else 120
columns; `-w N` overrides) without changing direction. A left-to-right diagram that is too
wide first gets its long labels word-wrapped, then continues on the next row, like wrapped
text:

```
                                                          ╭───────────╮
                                                        ┌►│ get quote ├────┐
┌─────────┐  ╭──────╮  ╭────────────╮  ╔════════╗ ┌─yes─┘ ╰───────────╯    │
│ request ├─►│ auth ├─►│ parse body ├─►║ valid? ╟─┤                        │
└─────────┘  ╰──────╯  ╰────────────╯  ╚════════╝ └─no──┐ ┌─────┐          │
                                                        └►│ 400 │          │
                                                          └─────┘          │
┌──────────────────────────────────────────────────────────────────────────┘
│  ╭──────────────╮  ╭─────────────╮  ╭──────────────╮
└─►│ create order ├─►│ charge card ├─►│ write ledger ├────┐
   ╰──────────────╯  ╰─────────────╯  ╰──────────────╯    │
┌─────────────────────────────────────────────────────────┘
│  ╭─────────────╮  ┌────────┐
└─►│ notify user ├─►│ 200 OK │
   ╰─────────────╯  └────────┘
```

When several edges cross the same break, the rows are joined by numbered connectors
(①, ②, ...) instead of lines. If a diagram still can't fit, drawtool prints a `note:` on
stderr. Top-down is a mode you choose (`flowchart TD` or `-d TD`), never a fallback.

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
