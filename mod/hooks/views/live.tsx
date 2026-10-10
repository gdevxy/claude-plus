import type { Line } from '../state'
import { PADDING } from '../state'
import type { Ui } from './widgets'
import { Row } from './widgets'

export type Tab = { key: string; hotkey: string; label: string; isActive: boolean; onPress: () => void }

export function Live({
  ui,
  width,
  lines,
  tabs,
  onWhy,
}: {
  ui: Ui
  width: number
  lines: Line[]
  tabs: Tab[]
  onWhy: (key: string) => void
}) {
  const { Box, Text, Button } = ui
  const inner = Math.max(1, width - 2 * PADDING)
  return (
    <Box flexDirection="column" width={width} paddingX={PADDING}>
      <Box key="header" width={inner} justifyContent="space-between">
        <Text color="claude" bold>
          ✻ claude-plus
        </Text>
        <Box>
          {tabs.map(tab => (
            <Button key={tab.key} hotkey={tab.hotkey} plain dimColor={!tab.isActive} label={tab.label} onPress={tab.onPress} />
          ))}
        </Box>
      </Box>
      <Text> </Text>
      {lines.map(line => (
        <Row ui={ui} line={line} width={inner} onWhy={onWhy} />
      ))}
    </Box>
  )
}
