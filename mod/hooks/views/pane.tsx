import { fit } from '../format'
import type { Palette } from '../palette'
import type { Line } from '../state'
import { PADDING } from '../state'
import type { Ui } from './widgets'
import { Row } from './widgets'

export const BRAND = '✻ claude-plus'
export const SHORT_BRAND = '✻'

export type Brand = { name: string; version: string | null }

export type Tab = { key: string; hotkey: string; label: string; short: string; isActive: boolean; onPress: () => void }

export type Choice = { text: string; isActive: boolean }

export type Control = { key: string; hotkey: string; label: string; choices: Choice[]; separator?: string; onPress: () => void }

export type ControlRow = { controls: Control[]; expanded: boolean }

const CONTROLS_GAP = 2

function separatorOf(control: Control): string {
  return control.separator ?? ' '
}

function choicesText(control: Control): string {
  return control.choices.map(choice => choice.text).join(separatorOf(control))
}

function choicesFit(inner: number, controls: readonly Control[]): boolean {
  const width = controls.reduce((sum, control) => sum + control.hotkey.length + 2 + choicesText(control).length, 0)
  return width + CONTROLS_GAP * Math.max(0, controls.length - 1) <= inner
}

export function controlRows(inner: number, controls: readonly Control[]): ControlRow[] {
  if (controls.length === 0) return []
  if (choicesFit(inner, controls)) return [{ controls: [...controls], expanded: true }]
  const rows: Control[][] = []
  let used = 0
  for (const control of controls) {
    const label = fit(control.label, Math.max(1, inner - control.hotkey.length - 2))
    const width = control.hotkey.length + 2 + label.length
    const last = rows[rows.length - 1]
    if (last !== undefined && used + CONTROLS_GAP + width <= inner) {
      last.push({ ...control, label })
      used += CONTROLS_GAP + width
    } else {
      rows.push([{ ...control, label }])
      used = width
    }
  }
  return rows.map(row => ({ controls: row, expanded: false }))
}

export function buttonsWidth(buttons: readonly { hotkey: string; label: string }[]): number {
  return buttons.reduce((sum, button) => sum + button.hotkey.length + 2 + button.label.length, 0) + Math.max(0, buttons.length - 1)
}

function brandWidth(brand: Brand): number {
  return brand.name.length + (brand.version === null ? 0 : 1 + brand.version.length)
}

export function brandFor(inner: number, tabs: readonly Tab[], version: string | null): Brand {
  const fits = (brand: Brand) => brandWidth(brand) + 1 + buttonsWidth(tabs) <= inner
  const candidates: Brand[] = [...(version === null ? [] : [{ name: BRAND, version }]), { name: BRAND, version: null }]
  return candidates.find(fits) ?? { name: SHORT_BRAND, version: null }
}

export function shownTabs(inner: number, tabs: readonly Tab[]): Tab[] {
  if (SHORT_BRAND.length + 1 + buttonsWidth(tabs) <= inner) return [...tabs]
  return tabs.map(tab => ({ ...tab, label: tab.short }))
}

export function tabRows(inner: number, brand: Brand, tabs: readonly Tab[]): Tab[][] {
  for (let count = 1; count < tabs.length; count += 1) {
    const size = Math.ceil(tabs.length / count)
    const rows = Array.from({ length: Math.ceil(tabs.length / size) }, (_, index) => tabs.slice(index * size, (index + 1) * size))
    if (rows.every((row, index) => buttonsWidth(row) + (index === 0 ? brandWidth(brand) + 1 : 0) <= inner)) return rows
  }
  return tabs.map(tab => [tab])
}

export function PaneBody({
  ui,
  width,
  lines,
  tabs,
  version = null,
  controls = [],
  palette = null,
  onToggle,
}: {
  ui: Ui
  width: number
  lines: Line[]
  tabs: Tab[]
  version?: string | null
  controls?: Control[]
  palette?: Palette | null
  onToggle: (key: string) => void
}) {
  const { Box, Text, Button } = ui
  const inner = Math.max(1, width - 2 * PADDING)
  const shown = shownTabs(inner, tabs)
  const brand = brandFor(inner, shown, version)
  const header = tabRows(inner, brand, shown)
  const rows = controlRows(inner, controls)
  return (
    <Box flexDirection="column" width={width} paddingX={PADDING}>
      {header.map((row, index) => (
        <Box key={index === 0 ? 'header' : `header-${index}`} width={inner} justifyContent={index === 0 ? 'space-between' : 'flex-end'}>
          {[
            ...(index === 0
              ? [
                  <Box key="identity" gap={1}>
                    <Text color="claude" bold>
                      {brand.name}
                    </Text>
                    {brand.version === null ? null : (
                      <Text dimColor>
                        {brand.version}
                      </Text>
                    )}
                  </Box>,
                ]
              : []),
            <Box key="tabs" gap={1}>
              {row.map(tab => (
                <Button key={tab.key} hotkey={tab.hotkey} plain dimColor={!tab.isActive} label={tab.label} onPress={tab.onPress} />
              ))}
            </Box>,
          ]}
        </Box>
      ))}
      <Text> </Text>
      {rows.length > 0
        ? [
            ...rows.map((row, index) => (
              <Box key={index === 0 ? 'controls' : `controls-${index}`} width={inner} gap={CONTROLS_GAP}>
                {row.controls.map(control =>
                  row.expanded ? (
                    <Button key={control.key} hotkey={control.hotkey} plain label={control.label} onPress={control.onPress}>
                      {control.choices.flatMap((choice, at) => [
                        ...(at > 0 ? [separatorOf(control)] : []),
                        <Text bold={choice.isActive} dimColor={!choice.isActive}>
                          {choice.text}
                        </Text>,
                      ])}
                    </Button>
                  ) : (
                    <Button key={control.key} hotkey={control.hotkey} plain label={control.label} onPress={control.onPress} />
                  ),
                )}
              </Box>
            )),
            <Text key="controls-gap"> </Text>,
          ]
        : []}
      {lines.map(line => (
        <Row ui={ui} line={line} width={inner} palette={palette} onToggle={onToggle} />
      ))}
    </Box>
  )
}
