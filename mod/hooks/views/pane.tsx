import type { Palette } from '../palette'
import type { Line } from '../state'
import { PADDING } from '../state'
import type { Ui } from './widgets'
import { Row } from './widgets'

export const BRAND = '✻ claude-plus'
export const SHORT_BRAND = '✻'

export type Tab = { key: string; hotkey: string; label: string; short: string; isActive: boolean; onPress: () => void }

export type Choice = { text: string; isActive: boolean }

export type Control = { key: string; hotkey: string; label: string; choices: Choice[]; onPress: () => void }

const CONTROLS_GAP = 2

function choicesText(choices: readonly Choice[]): string {
  return choices.map(choice => choice.text).join(' ')
}

function choicesFit(inner: number, controls: readonly Control[]): boolean {
  const width = controls.reduce((sum, control) => sum + control.hotkey.length + 2 + choicesText(control.choices).length, 0)
  return width + CONTROLS_GAP * Math.max(0, controls.length - 1) <= inner
}

export function buttonsWidth(buttons: readonly { hotkey: string; label: string }[]): number {
  return buttons.reduce((sum, button) => sum + button.hotkey.length + 2 + button.label.length, 0) + Math.max(0, buttons.length - 1)
}

export function brandFor(inner: number, tabs: readonly Tab[]): string {
  return BRAND.length + 1 + buttonsWidth(tabs) <= inner ? BRAND : SHORT_BRAND
}

export function shownTabs(inner: number, tabs: readonly Tab[]): Tab[] {
  if (SHORT_BRAND.length + 1 + buttonsWidth(tabs) <= inner) return [...tabs]
  return tabs.map(tab => ({ ...tab, label: tab.short }))
}

export function PaneBody({
  ui,
  width,
  lines,
  tabs,
  controls = [],
  palette = null,
  onToggle,
}: {
  ui: Ui
  width: number
  lines: Line[]
  tabs: Tab[]
  controls?: Control[]
  palette?: Palette | null
  onToggle: (key: string) => void
}) {
  const { Box, Text, Button } = ui
  const inner = Math.max(1, width - 2 * PADDING)
  const shown = shownTabs(inner, tabs)
  const expanded = choicesFit(inner, controls)
  return (
    <Box flexDirection="column" width={width} paddingX={PADDING}>
      <Box key="header" width={inner} justifyContent="space-between">
        <Text color="claude" bold>
          {brandFor(inner, shown)}
        </Text>
        <Box gap={1}>
          {shown.map(tab => (
            <Button key={tab.key} hotkey={tab.hotkey} plain dimColor={!tab.isActive} label={tab.label} onPress={tab.onPress} />
          ))}
        </Box>
      </Box>
      <Text> </Text>
      {controls.length > 0
        ? [
            <Box key="controls" width={inner} gap={CONTROLS_GAP}>
              {controls.map(control =>
                expanded ? (
                  <Button key={control.key} hotkey={control.hotkey} plain label={control.label} onPress={control.onPress}>
                    {control.choices.flatMap((choice, index) => [
                      ...(index > 0 ? [' '] : []),
                      <Text bold={choice.isActive} dimColor={!choice.isActive}>
                        {choice.text}
                      </Text>,
                    ])}
                  </Button>
                ) : (
                  <Button key={control.key} hotkey={control.hotkey} plain label={control.label} onPress={control.onPress} />
                ),
              )}
            </Box>,
            <Text key="controls-gap"> </Text>,
          ]
        : []}
      {lines.map(line => (
        <Row ui={ui} line={line} width={inner} palette={palette} onToggle={onToggle} />
      ))}
    </Box>
  )
}
