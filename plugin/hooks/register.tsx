import type { Register } from 'claude-code'

// A Bash call that runs drawtool, bare or by path (`~/.cargo/bin/drawtool`).
const DRAWTOOL = /^\s*(\S*\/)?drawtool(\s|$)/

const command = (input: unknown): string | undefined => {
  const c = (input as { command?: unknown } | undefined)?.command
  return typeof c === 'string' ? c : undefined
}

export const register: Register = on => {
  // ToolResult carries no input, so remember which calls ran drawtool.
  const drawn = new Set<string>()

  on('tool.call', { tool: 'Bash' }, ($, e, next) => {
    if (DRAWTOOL.test(e.command)) drawn.add(e.tool_use_id)
    return next(e)
  }).catch(($, e, next) => next(e)) // only display rides on this; never block the call

  // The call row: `drawtool (N-line flowchart LR)` in place of the whole heredoc.
  on('ui.render', { component: 'ToolUse' }, ($, e, next) => {
    const cmd = e.props.tool === 'Bash' ? command(e.props.input) : undefined
    if (cmd === undefined || !DRAWTOOL.test(cmd)) return next(e)
    drawn.add(e.props.tool_use_id)
    const lines = cmd.split('\n')
    const header = lines.find(l => /^\s*(flowchart|graph|stateDiagram(-v2)?|sequenceDiagram)\b/.test(l))?.trim()
    const short = `drawtool${header ? ` (${header}, ${lines.length - 2} lines)` : ''}`
    return next({ ...e, props: { ...e.props, input: { ...(e.props.input as object), command: short } } })
  })

  // The result: the whole diagram, never folded.
  on('ui.render', { component: 'ToolResult' }, ($, e, next) => {
    const out = e.props.output as { stdout?: unknown } | undefined
    if (e.props.isErrored || !drawn.has(e.props.tool_use_id) || typeof out?.stdout !== 'string') {
      return next(e)
    }
    const { Box, Text } = $.ui.resolve(e)
    return (
      <Box flexDirection="column">
        {out.stdout.replace(/\n+$/, '').split('\n').map((line, i) => (
          <Text key={String(i)}>{line}</Text>
        ))}
      </Box>
    )
  })
}
