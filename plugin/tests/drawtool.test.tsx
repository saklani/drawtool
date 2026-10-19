import { expect, test } from 'claude-code/testing'

const DIAGRAM = '┌───┐  ┌───┐\n│ a ├─►│ b │\n└───┘  └───┘\n'
const CMD = "drawtool <<'EOF'\nflowchart LR\na --> b\nEOF"

// Claude Code's own drawing, stubbed: one line naming what it was asked to draw.
const engine = (on: any) =>
  on('ui.render', ($: any, e: any) => {
    const { Text } = $.ui.resolve(e)
    return <Text>{`engine ${e.component}: ${e.props.input?.command ?? 'output'}`}</Text>
  })

test('a drawtool call: a short row, then every line of the diagram', async ($, on) => {
  engine(on)
  const id = 'toolu_draw'
  const row = await $.ui.mount({
    plugin: 'drawtool', surface: 'terminal', component: 'ToolUse', requestId: id,
    props: { tool_use_id: id, tool: 'Bash', input: { command: CMD }, isRunning: false, isErrored: false, isInterrupted: false },
  })
  expect(await row.find({ type: 'Text', text: 'engine ToolUse: drawtool (flowchart LR, 2 lines)' })).toBeDefined()
  await row.unmount()
  const ui = await $.ui.mount({
    plugin: 'drawtool', surface: 'terminal', component: 'ToolResult', requestId: id,
    props: { tool_use_id: id, tool: 'Bash', output: { stdout: DIAGRAM, stderr: '', interrupted: false }, isErrored: false },
  })
  expect(await ui.find({ type: 'Text', text: '│ a ├─►│ b │' })).toBeDefined()
  expect(await ui.find({ type: 'Text', text: '└───┘  └───┘' })).toBeDefined()
  await ui.unmount()
})

test('other Bash results are left to Claude Code', async ($, on) => {
  engine(on)
  const id = 'toolu_ls'
  const ui = await $.ui.mount({
    plugin: 'drawtool', surface: 'terminal', component: 'ToolResult', requestId: id,
    props: { tool_use_id: id, tool: 'Bash', output: { stdout: 'a\nb\n', stderr: '', interrupted: false }, isErrored: false },
  })
  expect(await ui.find({ type: 'Text', text: 'engine ToolResult: output' })).toBeDefined()
  await ui.unmount()
})
