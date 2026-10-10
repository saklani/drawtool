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
drawtool --highlight a,b,c diagram.mmd   # draw one path thick
drawtool --diff before.mmd after.mmd     # show what changed
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

## Sequence diagrams

Start the input with `sequenceDiagram` to draw who-calls-whom over time: participants across
the top, lifelines down, one message per row. Columns widen to fit each label, and `loop`,
`alt`/`else`, `opt` and `par`/`and` blocks become frames.

```
╭──────╮                   ┌─────┐                     ┌──────────┐  ┌──────────┐
│ user │                   │ API │                     │ Payments │  │ Postgres │
╰──┬───╯                   └──┬──┘                     └────┬─────┘  └────┬─────┘
   │                          │                             │             │
   │ POST /checkout           │                             │             │
   ├─────────────────────────►│                             │             │
   │                          │ create session              │             │
   │                          ├────────────────────────────►│             │
   │                          │ session url                 │             │
   │                          │◄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┤             │
   │                          │ insert order (pending)      │             │
   │                          ├─────────────────────────────│────────────►│
   │ redirect to session url  │                             │             │
   │◄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┤                             │             │
   │                          │ webhook: payment.succeeded  │             │
   │                          │◄────────────────────────────┤             │
   │                        ┌ alt signature ok ┄┄┄┄┄┄┄┄┄┄┄┄┄│┄┄┄┄┄┄┄┄┄┄┄┄┄│┄┐
   │                        ┆ │ mark order paid             │             │ ┆
   │                        ┆ ├─────────────────────────────│────────────►│ ┆
   │                        ├ else bad signature ┄┄┄┄┄┄┄┄┄┄┄│┄┄┄┄┄┄┄┄┄┄┄┄┄│┄┤
   │                        ┆ │ 400                         │             │ ┆
   │                        ┆ ├┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄x│             │ ┆
   │                        └┄│┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄│┄┄┄┄┄┄┄┄┄┄┄┄┄│┄┘
 ┌ loop until paid ┄┄┄┄┄┄┄┄┄┄┄│┄┐                           │             │
 ┆ │ GET /orders/:id          │ ┆                           │             │
 ┆ ├─────────────────────────►│ ┆                           │             │
 ┆ │ status                   │ ┆                           │             │
 ┆ │◄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┤ ┆                           │             │
 └┄│┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄│┄┘                           │             │
   │                          │                             │             │
╭──┴───╮                   ┌──┴──┐                     ┌────┴─────┐  ┌────┴─────┐
│ user │                   │ API │                     │ Payments │  │ Postgres │
╰──────╯                   └─────┘                     └──────────┘  └──────────┘
```

```
sequenceDiagram
actor user
participant api as API
participant pay as Payments
participant db as Postgres
user->>api: POST /checkout
api->>pay: create session
pay-->>api: session url
api->>db: insert order (pending)
api-->>user: redirect to session url
pay->>api: webhook: payment.succeeded
alt signature ok
api->>db: mark order paid
else bad signature
api--xpay: 400
end
loop until paid
user->>api: GET /orders/:id
api-->>user: status
end
```

Supported: `participant X as Name`, `actor X`; arrows `->>` (solid), `-->>` (dotted reply),
`->`/`-->` (no head), `-x`/`--x` (failed), `-)` (async); self-messages; `Note left of X`,
`Note right of X`, `Note over X,Y`; nested `loop`/`alt`/`else`/`opt`/`par`/`and` ... `end`.
`autonumber`, `activate`/`deactivate` and `title` are ignored. Long labels are word-wrapped to
fit the column budget.

## Groups

`subgraph` blocks are drawn as dashed frames. A frame only ever contains its own members;
groups that are different stages of the flow sit side by side.

```
                                              ┌ Vendors ┄┄┄┐
                                              ┆            ┆
                                              ┆ ╭────────╮ ┆
            ┌ API server ┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┐ ┌──►│ Stripe ├────webhook─┐
            ┆                             ┆ │ ┆ ╰────────╯ ┆          │
            ┆                  ╔═══════╗  ┆ │ ┆            ┆          │
            ┆                ┌►║ route ╟────┤ ┆ ╭────────╮ ┆          │
┌────────┐  ┆ ╭────────────╮ │ ╚═══════╝  ┆ └──►│ Bridge ├────webhook─┤
│ client ├───►│ middleware ├─┤            ┆   ┆ ╰────────╯ ┆          │
└────────┘  ┆ ╰────────────╯ └┄rate limit┄┄┄┐ ┆            ┆          │
            ┆                             ┆ ┆ └┄┄┄┄┄┄┄┄┄┄┄┄┘          │
            └┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┘ ┆                         │
                                            ┆ ┌ Storage ┄┄┄┄┄┄┄┄┄┄┄┄┄┄│┄┄┄┄┄┄┄┄┄┄┄┄┄┄┐
                                            ┆ ┆                       │              ┆
                                            ┆ ┆                       │ ╭──────────╮ ┆
                                            ┆ ┆                       └►│ Postgres │ ┆
                                            ┆ ┆                         ╰──────────╯ ┆
                                            ┆ ┆                                      ┆
                                            ┆ ┆                                      ┆
                                            ┆ ┆                                      ┆
                                            ┆ ┆ ╭───────╮                            ┆
                                            └┄┄►│ Redis │                            ┆
                                              ┆ ╰───────╯                            ┆
                                              ┆                                      ┆
                                              └┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┘
```

```
flowchart LR
subgraph api [API server]
  mw(middleware) --> route{route}
end
subgraph vendors [Vendors]
  stripe(Stripe)
  bridge(Bridge)
end
subgraph data [Storage]
  pg[(Postgres)]
  redis[(Redis)]
end
client[client] --> mw
mw -.->|rate limit| redis
route --> stripe & bridge
stripe & bridge -->|webhook| pg
```

## Highlighting a path

`--highlight req,auth,cache,ok` (or a `%% highlight: req, auth, cache, ok` line in the file)
draws those nodes with heavy borders and the edges between them thick, e.g. to show which
branch a request takes:

```
                                                                ╭──────────╮
                                                              ┌►│ postgres ├─┐
                                   ┏━━━━━━━━━┓          ┌─no──┘ ╰──────────╯ │ ┏━━━━━━━━┓
                                 ┌►┃ cached? ┠──────────┤                    ├►┃ 200 OK ┃
┏━━━━━━━━━┓  ┏━━━━━━━━━━━┓ ┌━yes━┘ ┗━━━━━━━━━┛          └━yes━━━━━━━━━━━━━━━━┘ ┗━━━━━━━━┛
┃ request ┠━►┃ token ok? ┠─┤
┗━━━━━━━━━┛  ┗━━━━━━━━━━━┛ └─no──┐ ┌──────────────────┐
                                 └►│ 401 Unauthorized │
                                   └──────────────────┘
```

## Before and after

`drawtool --diff before.mmd after.mmd` draws one diagram of how `before` became `after`,
matching nodes by id: `+` added (heavy), `-` removed (dashed/dotted), `~` relabelled.

```
                                                                       ╭──────────╮
                                     ┏━━━━━━━━━━━┓          ┌━+ no━━┬─►│ postgres ├─┐ ┌────────┐
                                   ┌►┃ + cached? ┠━━━━━━━━━━┤       ┆  ╰──────────╯ ├►│ 200 OK │
                                   ┃ ┗━━━━━━━━━━━┛          └━+ yes━┆┐              ┃ └────────┘
                           ┌━+ yes━┘                                ┆└━━━━━━━━━━━━━━┘
┌─────────┐  ╔═══════════╗ ┃         ┌────────────────────┐         ┆
│ request ├─►║ token ok? ╟─┼─no─────►│ ~ 401 Unauthorized │         ┆
└─────────┘  ╚═══════════╝ ┆         └────────────────────┘         ┆
                           └┄- yes┄┐                                ┆
                                   └┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┘
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
subgraph api [API server]     %% dashed group frame
  mw --> route
end
```

Shapes: `[box]`, `(rounded)`, `{decision}`, `([stadium])`, `[(database)]`, `[[sub]]`.
Cycles, self-loops and `subgraph ... end` groups (nested ones are flattened) are supported.
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
