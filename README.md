# drawtool

Render Mermaid-style flowcharts, sequence diagrams and UI wireframes as box diagrams in the terminal. Left-to-right by default,
so a diagram reads like a sentence and fits on screen without scrolling.

```
                                          ┌─────┐
                                        ┌►│ 401 │
┌────────────────┐  ╔═══════════╗ ┌─no──┘ └─────┘
│ GET /users/:id ├─►║ token ok? ╟─┤
└────────────────┘  ╚═══════════╝ └─yes─┐ ╔═════════╗ ┌─yes─────────────────────────────────────┐ ┌────────┐
                                        └►║ cached? ╟─┤                                         ├►│ 200 OK │
                                          ╚═════════╝ └─no──┐ ╒══════════╕       ╭────────────╮ │ └────────┘
                                                            └►│ postgres ├──row─►│ fill cache ├─┘
                                                              ╘══════════╛       ╰────────────╯
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
drawtool <<'EOF'
flowchart LR
a[build] --> b[ship]
EOF
```

drawtool is made for agents: the diagram always comes on stdin, as a quoted heredoc so
labels with `'`, `"`, `$` or backticks pass through untouched. Flags:

```sh
-d TD            # override direction (LR or TD)
-w 100           # column budget (default: terminal width, 0 = unlimited)
--highlight a,b,c   # draw one path thick
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

When several edges cross the same break, each gets its own line, nested so they never cross;
breaks are chosen where few edges cross. As in ELK's graph wrapping, where to break and how
much to wrap labels are decided up front from estimates that never undercount, and the whole
diagram is then laid out once. If a diagram still can't fit (a single layer wider than the
budget), drawtool prints a `note:` on stderr. Top-down is a mode you choose (`flowchart TD`
or `-d TD`), never a fallback; it only ever wraps labels.

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
            ┆ ╭────────────╮ ┌►║ route ╟────┤ ┆ ╭────────╮ ┆          │
┌────────┐  ┆ │            ├─┘ ╚═══════╝  ┆ └──►│ Bridge ├────webhook─┤
│ client ├───►│ middleware ├┄┐            ┆   ┆ ╰────────╯ ┆          │
└────────┘  ┆ │            │ └┄rate limit┄┄┄┐ ┆            ┆          │
            ┆ ╰────────────╯              ┆ ┆ └┄┄┄┄┄┄┄┄┄┄┄┄┘          │
            ┆                             ┆ ┆                         │
            └┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┘ ┆ ┌ Storage ┄┄┄┄┄┄┄┄┄┄┄┄┄┄│┄┄┄┄┄┄┄┄┄┄┄┄┄┄┐
                                            ┆ ┆                       │              ┆
                                            ┆ ┆                       │ ╒══════════╕ ┆
                                            ┆ ┆                       └►│ Postgres │ ┆
                                            ┆ ┆                         ╘══════════╛ ┆
                                            ┆ ┆                                      ┆
                                            ┆ ┆                                      ┆
                                            ┆ ┆                                      ┆
                                            ┆ ┆ ╒═══════╕                            ┆
                                            └┄┄►│ Redis │                            ┆
                                              ┆ ╘═══════╛                            ┆
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
                                                                ╒══════════╕
                                   ┏━━━━━━━━━┓                ┌►│ postgres ├─┐ ┏━━━━━━━━┓
                                   ┃         ┃          ┌─no──┘ ╘══════════╛ └►┃        ┃
             ┏━━━━━━━━━━━┓       ┌►┃ cached? ┠──────────┘                    ┌►┃ 200 OK ┃
┏━━━━━━━━━┓  ┃           ┃ ┌━yes━┘ ┃         ┠━━━━━━━━━━━━yes━━━━━━━━━━━━━━━━┘ ┃        ┃
┃ request ┠━►┃ token ok? ┠━┘       ┗━━━━━━━━━┛                                 ┗━━━━━━━━┛
┗━━━━━━━━━┛  ┃           ┠───no──┐
             ┗━━━━━━━━━━━┛       │ ┌──────────────────┐
                                 └►│ 401 Unauthorized │
                                   └──────────────────┘
```

## Stacking boxes

A `%% stack: a b c` line draws those boxes in one column, in that order (one row in TD), and
joins edges between neighbours with short straight arrows instead of routing them through the
layers. Use it to hang side effects under the step that causes them, or for a pair that talks
both ways, which would otherwise run back through the layers. Edges between boxes that aren't
neighbours in the stack run in a bracket beside it. Mermaid reads the line as a comment.

```
flowchart LR
api[POST /orders] --> validate --> save[save order] --> respond[201 Created]
save -->|write| cache[(cache)]
save -.->|async| audit[audit log]
%% stack: save cache audit
respond --> sync[sync worker]
sync -->|push| ledger[(ledger)]
ledger -->|ack| sync
%% stack: sync ledger
```

```
                                ┌────────────┐                            ┌─────────────┐
┌──────────────┐  ┌──────────┐  │            │          ┌─────────────┐ ┌►│ sync worker │
│ POST /orders ├─►│ validate ├─►│ save order ├─────────►│ 201 Created ├─┘ └──┬──────────┘
└──────────────┘  └──────────┘  │            ├┄┐        └─────────────┘ push │ ▲ ack
                                └───┬────────┘ ┆                             ▼ │
                                    │ write    ┆                          ╒════╧═══╕
                                    ▼          ┆                          │ ledger │
                                ╒═══════╕      ┆ async                    ╘════════╛
                                │ cache │      ┆
                                ╘═══════╛      ┆
                                               ┆
                                ┌───────────┐  ┆
                                │           │◄┄┘
                                │ audit log │
                                │           │
                                └───────────┘
```

## Wireframes

Start with `wireframe [title]` to sketch a screen instead of a flow. Elements are one per
line; `card`, `row` and `col` take indented children. Several `wireframe` blocks are drawn
side by side (wrapping to the column budget), e.g. to show a flow or before/after states.

```
┌─ Add money ──────────────────────────┐   ┌─ Home ───────────────────────────────┐
│ ← Back       Add money             ? │   │ ☰              Calm            Help │
├──────────────────────────────────────┤   ├──────────────────────────────────────┤
│                                      │   │ ┌─ Balance ────────────────────────┐ │
│ Amount ($)                           │   │ │            $1,240.00             │ │
│ ┌──────────────────────────────────┐ │   │ │ [ Add money ]           [ Send ] │ │
│ │ 0.00                             │ │   │ └──────────────────────────────────┘ │
│ └──────────────────────────────────┘ │   │   Activity     Cards      Rewards    │
│ [ $20 ]        [ $50 ]      [ $100 ] │   │ ──━━━━━━━━────────────────────────── │
│                                      │   │ • Coffee at Blue Bottle       -$4.50 │
│ (•) Card   ( ) Bank                  │   │ • Salary from Acme        +$3,200.00 │
│ Card                                 │   │ Round-up savings               (  ●) │
│ ┌──────────────────────────────────┐ │   ├──────────────────────────────────────┤
│ │ Visa •••• 4242                 ▾ │ │   │     Home        Pay       Profile    │
│ └──────────────────────────────────┘ │   │     ━━━━                             │
│ [x] Save for next time               │   └──────────────────────────────────────┘
│                                      │
│ ┏━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓ │
│ ┃             Continue             ┃ │
│ ┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛ │
└──────────────────────────────────────┘
```

```
wireframe Add money
  header: ← Back | Add money | ?
  spacer
  input: Amount ($) = 0.00
  row: [ $20 ] | [ $50 ] | [ $100 ]
  spacer
  radio: *Card | Bank
  select: Card = Visa •••• 4242
  checkbox: Save for next time = on
  spacer
  button: Continue

wireframe Home
  header: ☰ | Calm | Help
  card Balance:
    center: $1,240.00
    row: [ Add money ] | [ Send ]
  tabs: *Activity | Cards | Rewards
  - Coffee at Blue Bottle | -$4.50
  - Salary from Acme | +$3,200.00
  toggle: Round-up savings = on
  nav: *Home | Pay | Profile
```

| Element | Draws |
|---|---|
| `header: left \| centre \| right` | top bar with a divider under it |
| `text:` / `center:` / `right:` / plain line | text (wrapped) |
| `input: Label = placeholder`, `select: Label = value` | text field, dropdown |
| `button: Label` | full-width primary button; `[ Label ]` inline in text is a small one |
| `image: caption` | image placeholder |
| `card Title:` / `row:` / `col:` | bordered group / side-by-side columns / stack |
| `row: a \| b \| c` | inline columns (first left, last right, middle centred) |
| `tabs: *A \| B`, `nav: *Home \| Pay` | tab bar / bottom navigation, `*` = active |
| `toggle: Label = on`, `checkbox: Label = on`, `radio: *A \| B` | controls |
| `- item \| right` | list item, optional right-aligned part |
| `---`, `spacer` | divider, blank line |

## Syntax

A strict subset of Mermaid flowchart syntax: anything drawtool accepts means the same thing to
Mermaid, so a diagram renders the same on GitHub. (Checked against Mermaid 12.1's own parser on
thousands of generated diagrams.)

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
x <--> y                      %% arrowheads at both ends (also <-.-> <==>)
n[two<br>lines]               %% multi-line label
a[x] --> b; b --> c           %% `;` separates statements
subgraph api [API server]     %% dashed group frame
  mw --> route
end
%% stack: save cache
```

Shapes: `[box]`, `(rounded)`, `{decision}`, `([stadium])`, `[(database)]`, `[[sub]]`. Other Mermaid
shapes (`((circle))`, `{{hexagon}}`, `[/lean/]`, `>flag]`, ...) keep their label and are drawn as the
nearest of these. Cycles (a back edge runs back through the layers, its arrowhead at its
source's side, as in dot), self-loops and
`subgraph ... end` groups (nested ones are flattened) are supported. As in Mermaid, a node's last
label wins, `:::class`, `classDef`, `style`, `linkStyle` and `click` don't change the drawing, and a
node belongs to the innermost subgraph that mentions it.

Not supported, and rejected with an error rather than drawn wrong: `RL`/`BT` directions,
`--o`, `--x`, `o--o`, `x--x`, `~~~`, and edges to a subgraph. drawtool also accepts a few things Mermaid doesn't:
no `flowchart` line (drawn left-to-right), `%%` comments at the end of a line, `#` comment lines,
and `%% highlight: a, b` and `%% stack: a b` (plain comments to Mermaid).

In the output, `┼` `├` `┬` are joins (the lines connect); `╂` marks a crossing (they don't).

## Claude Code plugin

The `drawtool` plugin (in `plugin/`) has three parts:

- drawtool itself: on first use it downloads the prebuilt binary for your platform (macOS or
  Linux, arm64 or x86-64) from the [`build` release](https://github.com/saklani/drawtool/releases/tag/build),
  which CI rebuilds on every push that changes the crate. A `drawtool` already on your PATH wins.
- a skill that teaches Claude Code to sketch diagrams with drawtool when you ask it to explain a
  dataflow or control flow
- a display mod that shows each diagram in full where Claude ran it, under a short
  `drawtool (flowchart LR, N lines)` row, so Claude doesn't retype it into its reply

No separate install is needed. Type at a Claude Code prompt:

```
/plugin install drawtool --marketplace saklani/drawtool
```

Answer `y` to add the marketplace, then pick a scope. The mod needs Claude Code 2.1.287 or later.
On an older version the skill still works; ask Claude to paste the diagram if you can't see it.

The mod has its own tests: `claude plugin test plugin`.

## Tests

```sh
cargo test                    # examples/*.mmd must match tests/golden/
UPDATE_GOLDEN=1 cargo test    # accept intentional rendering changes
```

## License

MIT
